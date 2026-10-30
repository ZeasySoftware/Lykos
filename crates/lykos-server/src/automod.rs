//! AutoMod: Regeln pro Community, die der Server beim Erstellen und Bearbeiten von Nachrichten in
//! Community-Kanälen prüft (nicht in DMs).
//!
//! Einhängepunkt ist [`check`]: Jeder Pfad, der Nachrichten in einen Community-Kanal schreibt
//! (`routes::messages`, später Webhooks/Bots), ruft ihn **vor** dem Speichern mit dem endgültigen Inhalt auf.
//! Bei einem Verstoß mit Aktion „blockieren“ kommt `ApiError::AutomodBlocked` zurück (`403 automod_blocked`);
//! Alarm im Protokollkanal und Audit-Eintrag passieren als Nebenwirkung, auch wenn nicht blockiert wird.
//!
//! Regeln werden pro Community im Speicher gecacht (keine Datenbankabfrage pro Nachricht) und bei jeder
//! Änderung über die Verwaltung invalidiert. Der Cache lebt – wie `rate_limit.rs` – pro Serverprozess;
//! bei mehreren Instanzen müsste die Invalidierung über den Event-Bus laufen.

pub mod filters;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use self::filters::{FloodHit, FloodLimits, FloodTracker, KeywordFilter, LinkPolicy};
use crate::{
    audit,
    error::{ApiError, ApiResult},
    permissions::{MemberContext, Permissions},
    routes::messages,
    state::AppState,
};

/// Höchstlänge der eigenen Blockiermeldung.
pub const MAX_CUSTOM_MESSAGE_CHARS: usize = 150;
/// Höchstzahl der Ausnahmen (Rollen bzw. Kanäle) je Regel.
pub const MAX_EXEMPTIONS: usize = 100;
/// Gekürzter Nachrichteninhalt im Alarm.
const EXCERPT_CHARS: usize = 300;

/// Regelarten. Je Community gibt es jede Art höchstens einmal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Wortfilter.
    Keyword,
    /// Links außer erlaubten Domains.
    Links,
    /// Zu viele Erwähnungen.
    Mentions,
    /// Flut und Wiederholungen.
    Spam,
}

impl RuleKind {
    pub const ALL: [RuleKind; 4] = [RuleKind::Keyword, RuleKind::Links, RuleKind::Mentions, RuleKind::Spam];

    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Keyword => "keyword",
            RuleKind::Links => "links",
            RuleKind::Mentions => "mentions",
            RuleKind::Spam => "spam",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Anzeigename (deutsch).
    pub fn label(self) -> &'static str {
        match self {
            RuleKind::Keyword => "Wortfilter",
            RuleKind::Links => "Links blockieren",
            RuleKind::Mentions => "Erwähnungs-Spam",
            RuleKind::Spam => "Spam/Flut",
        }
    }

    /// Standardmeldung an den Absender, wenn keine eigene gesetzt ist.
    fn default_message(self) -> &'static str {
        match self {
            RuleKind::Keyword => {
                "Deine Nachricht enthält einen Begriff, der in dieser Community nicht erlaubt ist."
            }
            RuleKind::Links => "Links sind in dieser Community nur zu freigegebenen Seiten erlaubt.",
            RuleKind::Mentions => "Deine Nachricht erwähnt zu viele Personen auf einmal.",
            RuleKind::Spam => "Du schreibst zu schnell oder wiederholst dich. Warte einen Moment.",
        }
    }
}

/// Was bei einem Treffer passiert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Actions {
    /// Nachricht ablehnen (`403 automod_blocked`).
    pub block: bool,
    /// Alarm in den Protokollkanal.
    pub alert: bool,
    /// Eintrag ins Audit-Log der Community.
    pub audit: bool,
}

impl Default for Actions {
    fn default() -> Self {
        Self { block: true, alert: false, audit: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KeywordConfig {
    pub terms: Vec<String>,
    /// Ganze Wörter, die nie als Treffer zählen.
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinksConfig {
    pub allowed_domains: Vec<String>,
    pub block_foreign_invites: bool,
}

impl Default for LinksConfig {
    fn default() -> Self {
        Self { allowed_domains: Vec::new(), block_foreign_invites: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MentionsConfig {
    /// Mehr als so viele verschiedene Nutzer-Erwähnungen sind ein Verstoß.
    pub max_mentions: u32,
}

impl Default for MentionsConfig {
    fn default() -> Self {
        Self { max_mentions: 5 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpamConfig {
    /// Mehr als so viele Nachrichten in `per_seconds` Sekunden (0 = aus).
    pub max_messages: u32,
    pub per_seconds: u32,
    /// Mehr als so viele identische Nachrichten hintereinander (0 = aus).
    pub max_duplicates: u32,
}

impl Default for SpamConfig {
    fn default() -> Self {
        Self { max_messages: 5, per_seconds: 5, max_duplicates: 3 }
    }
}

/// Art-spezifische Einstellungen (in der API als `config`, passend zu `kind`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RuleConfig {
    Keyword(KeywordConfig),
    Links(LinksConfig),
    Mentions(MentionsConfig),
    Spam(SpamConfig),
}

impl RuleConfig {
    fn default_for(kind: RuleKind) -> Self {
        match kind {
            RuleKind::Keyword => RuleConfig::Keyword(Default::default()),
            RuleKind::Links => RuleConfig::Links(Default::default()),
            RuleKind::Mentions => RuleConfig::Mentions(Default::default()),
            RuleKind::Spam => RuleConfig::Spam(Default::default()),
        }
    }

    /// Aus JSON lesen; fehlende Felder bekommen Voreinstellungen.
    fn from_json(kind: RuleKind, value: Value) -> Result<Self, serde_json::Error> {
        let value = if value.is_null() { json!({}) } else { value };
        Ok(match kind {
            RuleKind::Keyword => RuleConfig::Keyword(serde_json::from_value(value)?),
            RuleKind::Links => RuleConfig::Links(serde_json::from_value(value)?),
            RuleKind::Mentions => RuleConfig::Mentions(serde_json::from_value(value)?),
            RuleKind::Spam => RuleConfig::Spam(serde_json::from_value(value)?),
        })
    }

    fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

/// Eine Regel, wie sie die API liefert und speichert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub kind: RuleKind,
    pub enabled: bool,
    pub actions: Actions,
    /// Protokollkanal für Alarme (Textkanal der Community).
    pub alert_channel_id: Option<Uuid>,
    /// Eigene Meldung an den Absender beim Blockieren.
    pub custom_message: Option<String>,
    pub exempt_role_ids: Vec<Uuid>,
    pub exempt_channel_ids: Vec<Uuid>,
    pub config: RuleConfig,
}

impl Rule {
    /// Voreinstellung: aus, blockieren, keine Ausnahmen.
    pub fn default_for(kind: RuleKind) -> Self {
        Self {
            kind,
            enabled: false,
            actions: Actions::default(),
            alert_channel_id: None,
            custom_message: None,
            exempt_role_ids: Vec::new(),
            exempt_channel_ids: Vec::new(),
            config: RuleConfig::default_for(kind),
        }
    }

    pub fn config_json(&self) -> Value {
        self.config.to_json()
    }
}

#[derive(FromRow)]
struct RuleRow {
    kind: String,
    enabled: bool,
    config: Value,
    action_block: bool,
    action_alert: bool,
    action_audit: bool,
    alert_channel_id: Option<Uuid>,
    custom_message: Option<String>,
    exempt_role_ids: Vec<Uuid>,
    exempt_channel_ids: Vec<Uuid>,
}

/// Alle vier Regeln der Community (fehlende mit Voreinstellungen), in fester Reihenfolge.
pub async fn load_rules(db: &PgPool, community_id: Uuid) -> ApiResult<Vec<Rule>> {
    let rows: Vec<RuleRow> = sqlx::query_as(
        "SELECT kind, enabled, config, action_block, action_alert, action_audit, alert_channel_id,
                custom_message, exempt_role_ids, exempt_channel_ids
         FROM automod_rules WHERE community_id = $1",
    )
    .bind(community_id)
    .fetch_all(db)
    .await?;
    let mut stored: HashMap<RuleKind, Rule> = HashMap::new();
    for row in rows {
        let Some(kind) = RuleKind::parse(&row.kind) else { continue };
        let config = RuleConfig::from_json(kind, row.config).unwrap_or_else(|e| {
            tracing::warn!(%community_id, kind = kind.as_str(), error = %e, "AutoMod-Konfiguration unlesbar");
            RuleConfig::default_for(kind)
        });
        stored.insert(
            kind,
            Rule {
                kind,
                enabled: row.enabled,
                actions: Actions {
                    block: row.action_block,
                    alert: row.action_alert,
                    audit: row.action_audit,
                },
                alert_channel_id: row.alert_channel_id,
                custom_message: row.custom_message,
                exempt_role_ids: row.exempt_role_ids,
                exempt_channel_ids: row.exempt_channel_ids,
                config,
            },
        );
    }
    Ok(RuleKind::ALL.into_iter().map(|k| stored.remove(&k).unwrap_or_else(|| Rule::default_for(k))).collect())
}

// --- Cache ------------------------------------------------------------------------------------

/// Vorbereitete Prüfung einer aktiven Regel.
#[derive(Debug)]
enum Check {
    Keyword(KeywordFilter),
    Links(LinkPolicy),
    Mentions(u32),
    Spam(FloodLimits),
}

#[derive(Debug)]
struct ActiveRule {
    rule: Rule,
    check: Check,
}

/// Aktive Regeln einer Community, fertig zum Prüfen (leer = AutoMod aus).
#[derive(Debug, Default)]
struct Compiled {
    rules: Vec<ActiveRule>,
}

impl Compiled {
    fn new(rules: Vec<Rule>, own_host: Option<&str>) -> Self {
        let rules = rules
            .into_iter()
            .filter(|r| r.enabled && (r.actions.block || r.actions.alert || r.actions.audit))
            .filter_map(|rule| {
                let check = match &rule.config {
                    RuleConfig::Keyword(c) => {
                        let filter = KeywordFilter::new(&c.terms, &c.allow);
                        if filter.is_empty() {
                            return None;
                        }
                        Check::Keyword(filter)
                    }
                    RuleConfig::Links(c) => Check::Links(LinkPolicy {
                        allowed_domains: c.allowed_domains.clone(),
                        block_foreign_invites: c.block_foreign_invites,
                        own_host: own_host.map(str::to_owned),
                    }),
                    RuleConfig::Mentions(c) => Check::Mentions(c.max_mentions),
                    RuleConfig::Spam(c) => Check::Spam(FloodLimits {
                        max_messages: c.max_messages,
                        per_seconds: c.per_seconds,
                        max_duplicates: c.max_duplicates,
                    }),
                };
                Some(ActiveRule { rule, check })
            })
            .collect();
        Self { rules }
    }
}

#[derive(Default)]
struct Inner {
    rules: RwLock<HashMap<Uuid, Arc<Compiled>>>,
    /// Zählt Invalidierungen – ein Ladevorgang, der eine Änderung überholt hat, schreibt nicht in den Cache.
    generation: AtomicU64,
    flood: Mutex<FloodTracker>,
}

/// AutoMod-Zustand im `AppState`: Regel-Cache und Flut-Zähler. Billig klonbar.
#[derive(Clone, Default)]
pub struct AutoMod {
    inner: Arc<Inner>,
}

impl AutoMod {
    /// Regeln der Community aus dem Cache, beim ersten Zugriff aus der Datenbank.
    async fn compiled(&self, state: &AppState, community_id: Uuid) -> ApiResult<Arc<Compiled>> {
        let cached = self.read_rules().get(&community_id).cloned();
        if let Some(c) = cached {
            return Ok(c);
        }
        let generation = self.inner.generation.load(Ordering::Acquire);
        let rules = load_rules(&state.db, community_id).await?;
        let own_host = filters::url_host(&state.config.public_url);
        let compiled = Arc::new(Compiled::new(rules, own_host.as_deref()));
        let mut cache = self.inner.rules.write().unwrap_or_else(|e| e.into_inner());
        if self.inner.generation.load(Ordering::Acquire) == generation {
            cache.insert(community_id, compiled.clone());
        }
        Ok(compiled)
    }

    fn read_rules(&self) -> std::sync::RwLockReadGuard<'_, HashMap<Uuid, Arc<Compiled>>> {
        self.inner.rules.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Regeln der Community verwerfen (nach Änderung über die Verwaltung oder Löschen der Community).
    pub fn invalidate(&self, community_id: Uuid) {
        let mut cache = self.inner.rules.write().unwrap_or_else(|e| e.into_inner());
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        cache.remove(&community_id);
    }

    fn flood_hit(
        &self,
        community: Uuid,
        author: Uuid,
        content: &str,
        limits: FloodLimits,
    ) -> Option<FloodHit> {
        let hash = filters::content_hash(content);
        let mut flood = self.inner.flood.lock().unwrap_or_else(|e| e.into_inner());
        flood.hit(community, author, Instant::now(), hash, limits)
    }
}

// --- Prüfung ---------------------------------------------------------------------------------

/// Wer schreibt: für Ausnahmen, Flut-Zähler, Alarm und Audit.
#[derive(Debug, Clone)]
pub struct Author {
    /// Nutzer-ID (Autor im Alarm, Akteur im Audit-Log, Schlüssel der Flut-Zähler). Für Webhooks/Bots ohne
    /// eigenes Konto z. B. die Webhook-ID als `flood_key` und `user_id = None`.
    pub user_id: Option<Uuid>,
    /// Schlüssel der Flut-Zähler (meist = `user_id`); `None` = keine Flut-Prüfung.
    pub flood_key: Option<Uuid>,
    /// Rollen des Autors (für Rollen-Ausnahmen).
    pub role_ids: Vec<Uuid>,
    /// Vom AutoMod ausgenommen (Besitzer, `ADMINISTRATOR`, `MANAGE_COMMUNITY`).
    pub bypass: bool,
}

impl Author {
    /// Mitglied einer Community: Besitzer und Mitglieder mit `ADMINISTRATOR`/`MANAGE_COMMUNITY` sind ausgenommen
    /// (`base()` liefert für Besitzer und Admins alle Rechte).
    pub fn member(user_id: Uuid, ctx: &MemberContext) -> Self {
        Self {
            user_id: Some(user_id),
            flood_key: Some(user_id),
            role_ids: ctx.roles.iter().map(|(id, _)| *id).collect(),
            bypass: ctx.base().contains(Permissions::MANAGE_COMMUNITY),
        }
    }

    /// Eingehender Webhook: ohne Konto und Rollen, nie ausgenommen; die Flut-Zähler laufen pro Webhook.
    pub fn webhook(webhook_id: Uuid) -> Self {
        Self { user_id: None, flood_key: Some(webhook_id), role_ids: vec![], bypass: false }
    }
}

/// Zu prüfende Nachricht (endgültiger Inhalt, so wie er gespeichert würde).
#[derive(Debug, Clone)]
pub struct Submission<'a> {
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub author: Author,
    /// Anzeigename, wenn ein Webhook schreibt (erscheint im Alarm als Absender, da es keinen Autor gibt).
    pub webhook_name: Option<&'a str>,
    pub content: &'a str,
    /// Bearbeitung einer bestehenden Nachricht (ohne Flut-Prüfung).
    pub edit: bool,
}

/// Treffer einer Regel.
#[derive(Debug)]
struct Hit<'r> {
    rule: &'r Rule,
    /// Begründung für Moderatoren (z. B. `Begriff „foo“`).
    detail: String,
    /// Alarm/Audit auslösen (bei Flut nur beim ersten Verstoß einer Serie).
    notify: bool,
}

/// Prüft eine Nachricht gegen die AutoMod-Regeln ihrer Community. `Err(AutomodBlocked)`, wenn eine Regel mit
/// Aktion „blockieren“ greift; Alarme und Audit-Einträge werden in jedem Trefferfall geschrieben.
/// Fehler beim Alarm/Audit werden nur protokolliert und halten die Nachricht nicht auf.
pub async fn check(state: &AppState, sub: Submission<'_>) -> ApiResult<()> {
    if sub.author.bypass {
        return Ok(());
    }
    let compiled = state.automod.compiled(state, sub.community_id).await?;
    if compiled.rules.is_empty() {
        return Ok(());
    }

    let mut hits = Vec::new();
    for active in &compiled.rules {
        let rule = &active.rule;
        if rule.exempt_channel_ids.contains(&sub.channel_id)
            || sub.author.role_ids.iter().any(|r| rule.exempt_role_ids.contains(r))
        {
            continue;
        }
        let hit = match &active.check {
            Check::Keyword(filter) => {
                filter.find(sub.content).map(|term| (format!("Begriff „{term}“"), true))
            }
            Check::Links(policy) => policy.violation(sub.content).map(|link| {
                let what = if link.invite { "Einladungslink" } else { "Link" };
                (format!("{what} zu {}", link.host), true)
            }),
            Check::Mentions(max) => mention_violation(state, sub.community_id, sub.content, *max)
                .await?
                .map(|n| (format!("{n} Erwähnungen (erlaubt: {max})"), true)),
            Check::Spam(limits) => match (sub.edit, sub.author.flood_key) {
                (false, Some(key)) => {
                    state.automod.flood_hit(sub.community_id, key, sub.content, *limits).map(|hit| {
                        let detail = match hit {
                            FloodHit::Rate { count, .. } => {
                                format!("{count} Nachrichten in {} s", limits.per_seconds)
                            }
                            FloodHit::Duplicate { count, .. } => format!("{count}× derselbe Text"),
                        };
                        (detail, hit.first())
                    })
                }
                _ => None,
            },
        };
        if let Some((detail, notify)) = hit {
            hits.push(Hit { rule, detail, notify });
        }
    }

    let blocking = hits.iter().find(|h| h.rule.actions.block);
    let blocked = blocking.is_some();
    for hit in hits.iter().filter(|h| h.notify) {
        if let Err(e) = notify(state, &sub, hit, blocked).await {
            tracing::warn!(error = %e, community_id = %sub.community_id, "AutoMod-Alarm fehlgeschlagen");
        }
    }
    match blocking {
        Some(hit) => Err(ApiError::AutomodBlocked {
            rule: hit.rule.kind.as_str(),
            rule_name: hit.rule.kind.label(),
            message: hit
                .rule
                .custom_message
                .clone()
                .unwrap_or_else(|| hit.rule.kind.default_message().to_owned()),
        }),
        None => Ok(()),
    }
}

/// Anzahl erwähnter Mitglieder, wenn sie `max` übersteigt. Die Datenbank wird nur gefragt, wenn schon die
/// Kandidaten im Text über der Grenze liegen (selten).
async fn mention_violation(
    state: &AppState,
    community_id: Uuid,
    content: &str,
    max: u32,
) -> ApiResult<Option<i64>> {
    let (candidates, names) = filters::mention_candidates(content);
    if candidates <= max as usize {
        return Ok(None);
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM members m JOIN users u ON u.id = m.user_id
         WHERE m.community_id = $1 AND lower(u.username) = ANY($2)",
    )
    .bind(community_id)
    .bind(&names)
    .fetch_one(&state.db)
    .await?;
    Ok((count > i64::from(max)).then_some(count))
}

/// Gekürzter Inhalt für den Alarm.
fn excerpt(content: &str) -> String {
    let mut chars = content.chars();
    let mut out: String = chars.by_ref().take(EXCERPT_CHARS).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

/// Alarm in den Protokollkanal und/oder Audit-Eintrag für einen Treffer.
async fn notify(state: &AppState, sub: &Submission<'_>, hit: &Hit<'_>, blocked: bool) -> ApiResult<()> {
    let rule = hit.rule;
    if rule.actions.alert
        && let Some(alert_channel) = rule.alert_channel_id
    {
        let content = json!({
            "rule": rule.kind.as_str(),
            "rule_name": rule.kind.label(),
            "channel_id": sub.channel_id,
            "blocked": blocked,
            "edit": sub.edit,
            "detail": hit.detail,
            "excerpt": excerpt(sub.content),
        })
        .to_string();
        let id = Uuid::now_v7();
        // Nur in einen Textkanal derselben Community (der Cache kann einen inzwischen gelöschten Kanal kennen).
        // Bei Webhooks nur der Name (ohne `webhook_id`): der Alarm gehört nicht dem Webhook und lässt sich
        // über dessen Token weder bearbeiten noch löschen.
        let inserted = sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, kind, webhook_name)
             SELECT $1, c.id, $4, $5, 'automod_alert', $6 FROM channels c
             WHERE c.id = $2 AND c.community_id = $3 AND c.kind = 'text'",
        )
        .bind(id)
        .bind(alert_channel)
        .bind(sub.community_id)
        .bind(sub.author.user_id)
        .bind(&content)
        .bind(sub.webhook_name)
        .execute(&state.db)
        .await?
        .rows_affected();
        if inserted > 0 {
            messages::broadcast(state, "MESSAGE_CREATE", id).await?;
        }
    }
    if rule.actions.audit
        && let Some(actor) = sub.author.user_id
    {
        // Ohne Nachrichteninhalt (wie beim Löschen durch Moderatoren).
        audit::record(
            &state.db,
            sub.community_id,
            actor,
            if blocked { "automod.block" } else { "automod.flag" },
            None,
            json!({
                "rule": rule.kind.as_str(),
                "channel_id": sub.channel_id,
                "edit": sub.edit,
                "detail": hit.detail,
            }),
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_kinds_roundtrip() {
        for kind in RuleKind::ALL {
            assert_eq!(RuleKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(RuleKind::parse("timeout"), None);
    }

    #[test]
    fn config_defaults_and_partial_json() {
        let RuleConfig::Spam(spam) =
            RuleConfig::from_json(RuleKind::Spam, json!({ "max_messages": 8 })).unwrap()
        else {
            panic!()
        };
        assert_eq!(spam, SpamConfig { max_messages: 8, per_seconds: 5, max_duplicates: 3 });
        let RuleConfig::Links(links) = RuleConfig::from_json(RuleKind::Links, Value::Null).unwrap() else {
            panic!()
        };
        assert!(links.block_foreign_invites);
        let rule = Rule::default_for(RuleKind::Keyword);
        assert!(!rule.enabled && rule.actions.block && !rule.actions.alert);
        assert_eq!(rule.config_json(), json!({ "terms": [], "allow": [] }));
    }

    #[test]
    fn compiled_skips_disabled_and_empty_rules() {
        let mut keyword = Rule::default_for(RuleKind::Keyword);
        keyword.enabled = true; // ohne Begriffe → nichts zu prüfen
        let mut mentions = Rule::default_for(RuleKind::Mentions);
        mentions.enabled = true;
        let mut no_action = Rule::default_for(RuleKind::Spam);
        no_action.enabled = true;
        no_action.actions = Actions { block: false, alert: false, audit: false };
        let compiled =
            Compiled::new(vec![keyword, mentions, no_action, Rule::default_for(RuleKind::Links)], None);
        assert_eq!(compiled.rules.len(), 1);
        assert_eq!(compiled.rules[0].rule.kind, RuleKind::Mentions);
    }

    #[test]
    fn excerpt_is_shortened() {
        assert_eq!(excerpt("kurz"), "kurz");
        let long = "ä".repeat(EXCERPT_CHARS + 5);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), EXCERPT_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
