//! Import von Discord-Servervorlagen (`https://discord.new/CODE`).
//!
//! Der Server holt die Vorlage selbst (nicht der Browser: CSP, und Discord sieht so keine Nutzer-IPs) – aber
//! nur über einen festen API-Endpunkt (`Config::discord_api_url`) mit einem geprüften Code; Nutzer können keine
//! beliebige URL abrufen lassen. Ablauf: `extract_code` → `fetch` (Abruf, Größen-/Zeitlimit) → `parse`
//! (reine Abbildung auf Lykos, ohne Netz testbar) → `apply` (alles in der Transaktion des Community-Anlegens).
//!
//! Übernommen werden Rollen (Name, Farbe, Reihenfolge, Rechte), Kategorien, Text- und Sprachkanäle (Thema,
//! Position, Nutzerlimit, Bitrate), Rechte-Overrides je Rolle, Systemkanal und Beschreibung. Was Lykos nicht
//! kennt, landet als Hinweis in `Template::notes`.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    permissions::{self, Permissions},
    routes::{channels, communities, customization},
};

/// Öffentliche Discord-API (Vorlagen sind ohne Anmeldung abrufbar).
pub const DEFAULT_API_URL: &str = "https://discord.com/api/v10";
/// Länge eines Vorlagen-Codes (Discord nutzt 12 Zeichen; großzügig, aber begrenzt).
const CODE_LEN: std::ops::RangeInclusive<usize> = 2..=64;
/// Hosts, deren Links als Vorlagen-Link gelten.
const HOSTS: &[&str] =
    &["discord.new", "discord.com", "www.discord.com", "discordapp.com", "www.discordapp.com", "discord.gg"];
/// Abbruch des Abrufs nach dieser Zeit.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Höchstgröße der Antwort (die Fixture mit 51 Kanälen hat ≈ 34 KB).
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Harte Obergrenzen (Discord selbst erlaubt 250 Rollen und 500 Kanäle).
pub const MAX_ROLES: usize = 250;
pub const MAX_CHANNELS: usize = 500;
/// Höchstlänge von Kanal-/Rollen-/Kategorienamen, Themen und Community-Namen in Lykos.
const NAME_MAX: usize = 64;
const TOPIC_MAX: usize = 1024;
const COMMUNITY_NAME_MAX: usize = 100;

/// Vorlagen-Code aus einem eingefügten Text: nackter Code oder Link auf `discord.new/CODE`,
/// `discord.com/template/CODE` (auch `discordapp.com`, `discord.gg`). Fremde Hosts und alles andere → `None`.
pub fn extract_code(input: &str) -> Option<String> {
    let input = input.trim();
    if is_valid_code(input) {
        return Some(input.to_owned());
    }
    let with_scheme = if input.contains("://") { input.to_owned() } else { format!("https://{input}") };
    let url = reqwest::Url::parse(&with_scheme).ok()?;
    if !matches!(url.scheme(), "https" | "http") || url.port().is_some() || !url.username().is_empty() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if !HOSTS.contains(&host.as_str()) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let code = match (host.as_str(), segments.as_slice()) {
        ("discord.new", [code]) => *code,
        (_, ["template" | "templates", code]) => *code,
        _ => return None,
    };
    is_valid_code(code).then(|| code.to_owned())
}

fn is_valid_code(code: &str) -> bool {
    CODE_LEN.contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Ergebnis der Abbildung einer Vorlage auf Lykos (Reihenfolgen bereits wie in Lykos).
#[derive(Debug, Clone)]
pub struct Template {
    pub code: String,
    /// Vorschlag für den Community-Namen (Name der Vorlage).
    pub name: String,
    pub description: Option<String>,
    /// Rechte der Standardrolle (aus `@everyone`).
    pub default_permissions: Permissions,
    /// Rollen ohne `@everyone`, höchste zuerst (Lykos-Position 1, 2, …).
    pub roles: Vec<TemplateRole>,
    /// Kategorien in Anzeige-Reihenfolge.
    pub categories: Vec<TemplateCategory>,
    /// Kanäle in Anzeige-Reihenfolge (ohne Kategorie zuerst, je Gruppe Text vor Sprache).
    pub channels: Vec<TemplateChannel>,
    /// Index des Systemkanals in `channels` (nur Textkanäle).
    pub system_channel: Option<usize>,
    /// Index des AFK-Kanals in `channels` (nur Sprachkanäle) und Wartezeit in Sekunden (auf die Lykos-Stufen
    /// gerundet).
    pub afk_channel: Option<usize>,
    pub afk_timeout: Option<i32>,
    /// Hinweise, was nicht (genau so) übernommen wird.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TemplateRole {
    pub name: String,
    pub permissions: Permissions,
    /// 0xRRGGBB; `None` = ohne Farbe (Discord: 0).
    pub color: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct TemplateCategory {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TemplateChannel {
    pub name: String,
    /// `text` oder `voice`.
    pub kind: &'static str,
    pub topic: Option<String>,
    pub position: i32,
    /// Index in `Template::categories`.
    pub category: Option<usize>,
    /// Nur Sprachkanäle: Bitrate in kbit/s (32–256) und Nutzerlimit (1–99).
    pub audio_bitrate: i32,
    pub user_limit: Option<i32>,
    pub overrides: Vec<TemplateOverride>,
    /// War bei Discord ein Forum-/Medienkanal.
    pub from_forum: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleRef {
    /// Standardrolle (`@everyone`).
    Default,
    /// Index in `Template::roles`.
    Role(usize),
}

#[derive(Debug, Clone, Copy)]
pub struct TemplateOverride {
    pub role: RoleRef,
    pub allow: Permissions,
    pub deny: Permissions,
}

// --- Rohformat der Discord-API ---------------------------------------------------------------------

/// Zahl oder Dezimal-String (Discord liefert IDs in Vorlagen als kleine Zahlen, Rechte als Strings).
#[derive(Deserialize)]
#[serde(untagged)]
enum NumOrStr {
    Num(u64),
    Str(String),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(try_from = "NumOrStr")]
struct U64(u64);

impl TryFrom<NumOrStr> for U64 {
    type Error = String;
    fn try_from(v: NumOrStr) -> Result<Self, String> {
        match v {
            NumOrStr::Num(n) => Ok(Self(n)),
            NumOrStr::Str(s) => s.trim().parse().map(Self).map_err(|_| format!("keine Zahl: {s}")),
        }
    }
}

#[derive(Deserialize)]
struct RawTemplate {
    code: String,
    #[serde(default)]
    name: String,
    description: Option<String>,
    serialized_source_guild: RawGuild,
}

#[derive(Deserialize)]
struct RawGuild {
    #[serde(default)]
    name: String,
    description: Option<String>,
    afk_channel_id: Option<U64>,
    afk_timeout: Option<i64>,
    system_channel_id: Option<U64>,
    #[serde(default)]
    roles: Vec<RawRole>,
    #[serde(default)]
    channels: Vec<RawChannel>,
}

#[derive(Deserialize)]
struct RawRole {
    id: U64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    permissions: U64,
    #[serde(default)]
    color: i64,
    #[serde(default)]
    hoist: bool,
    #[serde(default)]
    mentionable: bool,
}

#[derive(Deserialize)]
struct RawChannel {
    id: U64,
    #[serde(rename = "type")]
    kind: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    position: i64,
    topic: Option<String>,
    bitrate: Option<i64>,
    user_limit: Option<i64>,
    parent_id: Option<U64>,
    #[serde(default)]
    nsfw: bool,
    rate_limit_per_user: Option<i64>,
    #[serde(default)]
    permission_overwrites: Vec<RawOverwrite>,
}

#[derive(Deserialize)]
struct RawOverwrite {
    id: U64,
    /// 0/`"role"` = Rolle, 1/`"member"` = Mitglied.
    #[serde(rename = "type")]
    kind: NumOrStr,
    #[serde(default)]
    allow: U64,
    #[serde(default)]
    deny: U64,
}

impl RawOverwrite {
    fn is_role(&self) -> bool {
        match &self.kind {
            NumOrStr::Num(n) => *n == 0,
            NumOrStr::Str(s) => s == "0" || s.eq_ignore_ascii_case("role"),
        }
    }
}

// --- Abbildung -------------------------------------------------------------------------------------

/// Steuerzeichen raus, trimmen, auf `max` Zeichen kürzen.
fn clip(text: &str, max: usize) -> String {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    let clipped: String = cleaned.trim().chars().take(max).collect();
    clipped.trim().to_owned()
}

fn clip_opt(text: Option<&str>, max: usize) -> Option<String> {
    text.map(|t| clip(t, max)).filter(|t| !t.is_empty())
}

/// Kanalname nach Lykos-Regeln (Textkanäle klein, Leerzeichen → `-`; 1–64 Zeichen).
fn channel_name(name: &str, kind: &str) -> String {
    let cleaned = clip(name, usize::MAX);
    let fallback = if kind == "text" { "kanal" } else { "Sprachkanal" };
    // Erst kürzen, dann normalisieren – `normalize_channel_name` lehnt zu lange Namen ab statt zu kürzen.
    let clipped = clip(&cleaned, NAME_MAX);
    match channels::normalize_channel_name(&clipped, kind) {
        Ok(normalized) => clip(&normalized, NAME_MAX),
        Err(_) => fallback.to_owned(),
    }
}

fn name_or(name: &str, max: usize, fallback: &str) -> String {
    let n = clip(name, max);
    if n.is_empty() { fallback.to_owned() } else { n }
}

enum Kind {
    Text { forum: bool },
    Voice,
    Category,
    Unsupported,
}

fn kind_of(discord_type: i64) -> Kind {
    match discord_type {
        0 | 5 => Kind::Text { forum: false },
        15 | 16 => Kind::Text { forum: true },
        2 | 13 => Kind::Voice,
        4 => Kind::Category,
        _ => Kind::Unsupported,
    }
}

/// JSON-Antwort von `GET /guilds/templates/{code}` auf Lykos abbilden. Fehler → `invalid_template`.
pub fn parse(json: &[u8]) -> ApiResult<Template> {
    let raw: RawTemplate = serde_json::from_slice(json).map_err(|e| {
        tracing::info!(error = %e, "Discord-Vorlage nicht lesbar");
        ApiError::BadRequest("invalid_template")
    })?;
    build(raw)
}

fn build(raw: RawTemplate) -> ApiResult<Template> {
    let guild = raw.serialized_source_guild;
    if guild.roles.len() > MAX_ROLES || guild.channels.len() > MAX_CHANNELS {
        return Err(ApiError::BadRequest("invalid_template"));
    }
    let mut notes = Vec::new();
    let mut unmapped_bits = 0u64;

    // Rollen: Discord liefert sie aufsteigend nach Hierarchie, `@everyone` (ID 0) zuerst.
    let everyone_index = guild.roles.iter().position(|r| r.id.0 == 0).unwrap_or(0);
    let default_permissions = match guild.roles.get(everyone_index) {
        Some(r) => {
            unmapped_bits |= permissions::discord_unmapped(r.permissions.0);
            permissions::from_discord(r.permissions.0)
        }
        None => Permissions::DEFAULT_ROLE,
    };
    let mut roles = Vec::new();
    // Discord-ID → Index in `roles`.
    let mut role_index: HashMap<u64, usize> = HashMap::new();
    let mut role_options = false;
    for (i, r) in guild.roles.iter().enumerate().rev() {
        if i == everyone_index || role_index.contains_key(&r.id.0) {
            continue;
        }
        unmapped_bits |= permissions::discord_unmapped(r.permissions.0);
        role_options |= r.hoist || r.mentionable;
        role_index.insert(r.id.0, roles.len());
        roles.push(TemplateRole {
            name: name_or(&r.name, NAME_MAX, "Rolle"),
            permissions: permissions::from_discord(r.permissions.0),
            color: (r.color > 0 && r.color <= 0xFF_FFFF).then_some(r.color as i32),
        });
    }
    let everyone_id = guild.roles.get(everyone_index).map(|r| r.id.0);

    // Kategorien nach (Position, ID).
    let mut raw_categories: Vec<&RawChannel> =
        guild.channels.iter().filter(|c| matches!(kind_of(c.kind), Kind::Category)).collect();
    raw_categories.sort_by_key(|c| (c.position, c.id.0));
    let category_index: HashMap<u64, usize> =
        raw_categories.iter().enumerate().map(|(i, c)| (c.id.0, i)).collect();
    let categories: Vec<TemplateCategory> = raw_categories
        .iter()
        .map(|c| TemplateCategory { name: name_or(&c.name, NAME_MAX, "Kategorie") })
        .collect();
    if raw_categories.iter().any(|c| !c.permission_overwrites.is_empty()) {
        notes.push(
            "Rechte an Kategorien übernimmt Lykos nicht – die Kanäle behalten ihre eigenen Rechte."
                .to_owned(),
        );
    }

    // Kanäle: ohne Kategorie zuerst, dann je Kategorie; innerhalb Text vor Sprache, dann Position.
    let mut skipped = 0;
    let mut forums = 0;
    let mut member_overrides = false;
    let mut slowmode_or_nsfw = false;
    let mut sorted: Vec<(&RawChannel, &'static str, bool)> = Vec::new();
    for c in &guild.channels {
        match kind_of(c.kind) {
            Kind::Text { forum } => sorted.push((c, "text", forum)),
            Kind::Voice => sorted.push((c, "voice", false)),
            Kind::Category => {}
            Kind::Unsupported => skipped += 1,
        }
    }
    let group = |c: &RawChannel| c.parent_id.and_then(|p| category_index.get(&p.0).copied());
    sorted.sort_by_key(|(c, kind, _)| {
        (group(c).map_or(-1, |g| g as i64), *kind == "voice", c.position, c.id.0)
    });

    let mut channels = Vec::new();
    let mut channel_index: HashMap<u64, usize> = HashMap::new();
    for (position, (c, kind, forum)) in sorted.into_iter().enumerate() {
        forums += usize::from(forum);
        slowmode_or_nsfw |= c.nsfw || c.rate_limit_per_user.is_some_and(|r| r > 0);
        let mut overrides = Vec::new();
        for o in &c.permission_overwrites {
            if !o.is_role() {
                member_overrides = true;
                continue;
            }
            let role = if Some(o.id.0) == everyone_id {
                RoleRef::Default
            } else if let Some(i) = role_index.get(&o.id.0) {
                RoleRef::Role(*i)
            } else {
                // Vorlagen enthalten Overrides gelöschter Rollen – ohne Rolle nichts zu übernehmen.
                continue;
            };
            unmapped_bits |= permissions::discord_unmapped(o.allow.0 | o.deny.0);
            let allow = permissions::from_discord(o.allow.0);
            let deny = permissions::from_discord(o.deny.0) & !allow;
            if (allow | deny) == Permissions::NONE
                || overrides.iter().any(|x: &TemplateOverride| x.role == role)
            {
                continue;
            }
            overrides.push(TemplateOverride { role, allow, deny });
        }
        let voice = kind == "voice";
        channel_index.insert(c.id.0, channels.len());
        channels.push(TemplateChannel {
            name: channel_name(&c.name, kind),
            kind,
            topic: clip_opt(c.topic.as_deref(), TOPIC_MAX),
            position: i32::try_from(position).unwrap_or(i32::MAX),
            category: group(c),
            audio_bitrate: if voice {
                let kbit = c.bitrate.unwrap_or(64_000) / 1000;
                kbit.clamp(
                    i64::from(*channels::AUDIO_BITRATE_RANGE.start()),
                    i64::from(*channels::AUDIO_BITRATE_RANGE.end()),
                ) as i32
            } else {
                64
            },
            user_limit: if voice {
                c.user_limit.filter(|l| *l > 0).map(|l| {
                    l.clamp(
                        i64::from(*channels::USER_LIMIT_RANGE.start()),
                        i64::from(*channels::USER_LIMIT_RANGE.end()),
                    ) as i32
                })
            } else {
                None
            },
            overrides,
            from_forum: forum,
        });
    }

    let system_channel = guild
        .system_channel_id
        .and_then(|id| channel_index.get(&id.0).copied())
        .filter(|i| channels[*i].kind == "text");
    let afk_channel = guild
        .afk_channel_id
        .and_then(|id| channel_index.get(&id.0).copied())
        .filter(|i| channels[*i].kind == "voice");
    // Lykos kennt dieselben Stufen wie Discord; Abweichendes rundet auf die nächste Stufe auf.
    let afk_timeout = guild.afk_timeout.filter(|t| *t > 0).map(|t| {
        communities::AFK_TIMEOUTS
            .iter()
            .copied()
            .find(|&s| i64::from(s) >= t)
            .unwrap_or(communities::AFK_TIMEOUTS[communities::AFK_TIMEOUTS.len() - 1])
    });

    if forums > 0 {
        notes.push(format!(
            "{forums} Forum- bzw. Medienkan{} wird als Textkanal angelegt.",
            if forums == 1 { "al" } else { "äle" }
        ));
    }
    if skipped > 0 {
        notes.push(format!("{skipped} Kanäle mit nicht unterstütztem Typ werden übersprungen."));
    }
    if unmapped_bits != 0 {
        notes.push(
            "Rechte ohne Entsprechung in Lykos (z. B. Threads, Events, Prioritätssprecher) werden verworfen."
                .to_owned(),
        );
    }
    if member_overrides {
        notes.push("Rechte für einzelne Mitglieder werden nicht übernommen.".to_owned());
    }
    if role_options {
        notes.push("Rollen-Optionen „getrennt anzeigen“ und „erwähnbar“ gibt es in Lykos nicht.".to_owned());
    }
    if slowmode_or_nsfw {
        notes.push("Slowmode und Altersbeschränkung von Kanälen gibt es in Lykos nicht.".to_owned());
    }

    let name = if raw.name.trim().is_empty() { &guild.name } else { &raw.name };
    Ok(Template {
        code: raw.code,
        name: name_or(name, COMMUNITY_NAME_MAX, "Community"),
        description: clip_opt(
            guild.description.as_deref().or(raw.description.as_deref()),
            customization::DESCRIPTION_MAX_CHARS,
        ),
        default_permissions,
        roles,
        categories,
        channels,
        system_channel,
        afk_channel,
        afk_timeout,
        notes,
    })
}

// --- Abruf -----------------------------------------------------------------------------------------

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::install_crypto_provider();
        reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            // Keinen Weiterleitungen folgen: abgefragt wird ausschließlich der konfigurierte Endpunkt.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("lykos-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP-Client")
    })
}

/// Vorlage `code` von der Discord-API (`api_url`, siehe `Config::discord_api_url`) holen und abbilden.
/// `code` muss aus `extract_code` stammen. Fehler: `template_not_found`, `template_fetch_failed`,
/// `invalid_template`.
pub async fn fetch(api_url: &str, code: &str) -> ApiResult<Template> {
    if !is_valid_code(code) {
        return Err(ApiError::BadRequest("invalid_template_code"));
    }
    let url = format!("{}/guilds/templates/{code}", api_url.trim_end_matches('/'));
    let failed = |e: &dyn std::fmt::Display| {
        tracing::warn!(error = %e, "Discord-Vorlage nicht abrufbar");
        ApiError::ServiceUnavailable("template_fetch_failed")
    };
    let res = http().get(&url).send().await.map_err(|e| failed(&e))?;
    match res.status() {
        s if s.is_success() => {}
        // Unbekannte Vorlage beantwortet Discord mit 404 (bei kaputten Codes teils 400).
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::BAD_REQUEST => {
            return Err(ApiError::NotFound("template_not_found"));
        }
        s => return Err(failed(&s)),
    }
    let body = crate::desktop::read_limited(res, MAX_RESPONSE_BYTES).await.map_err(|e| failed(&e))?;
    parse(&body)
}

// --- Vorschau --------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Preview {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub counts: Counts,
    pub roles: Vec<PreviewRole>,
    /// Kanäle ohne Kategorie.
    pub channels: Vec<PreviewChannel>,
    pub categories: Vec<PreviewCategory>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Counts {
    pub roles: usize,
    pub categories: usize,
    pub text_channels: usize,
    pub voice_channels: usize,
}

#[derive(Debug, Serialize)]
pub struct PreviewRole {
    pub name: String,
    pub color: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct PreviewChannel {
    pub name: String,
    pub kind: &'static str,
    /// Bei Discord ein Forum-/Medienkanal (wird Textkanal).
    pub from_forum: bool,
}

#[derive(Debug, Serialize)]
pub struct PreviewCategory {
    pub name: String,
    pub channels: Vec<PreviewChannel>,
}

impl Template {
    pub fn preview(&self) -> Preview {
        let channel = |c: &TemplateChannel| PreviewChannel {
            name: c.name.clone(),
            kind: c.kind,
            from_forum: c.from_forum,
        };
        Preview {
            code: self.code.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            counts: Counts {
                roles: self.roles.len(),
                categories: self.categories.len(),
                text_channels: self.channels.iter().filter(|c| c.kind == "text").count(),
                voice_channels: self.channels.iter().filter(|c| c.kind == "voice").count(),
            },
            roles: self.roles.iter().map(|r| PreviewRole { name: r.name.clone(), color: r.color }).collect(),
            channels: self.channels.iter().filter(|c| c.category.is_none()).map(channel).collect(),
            categories: self
                .categories
                .iter()
                .enumerate()
                .map(|(i, cat)| PreviewCategory {
                    name: cat.name.clone(),
                    channels: self.channels.iter().filter(|c| c.category == Some(i)).map(channel).collect(),
                })
                .collect(),
            notes: self.notes.clone(),
        }
    }
}

// --- Anlegen ---------------------------------------------------------------------------------------

/// Rollen, Kategorien, Kanäle, Overrides und Systemkanal der Vorlage in die frisch angelegte Community
/// schreiben (innerhalb der Transaktion des Anlegens). Community, Standardrolle (mit
/// `default_permissions`) und Besitzer-Mitgliedschaft legt der Aufrufer an.
pub async fn apply(
    tx: &mut PgConnection,
    community_id: Uuid,
    default_role_id: Uuid,
    template: &Template,
) -> ApiResult<()> {
    let mut role_ids = Vec::with_capacity(template.roles.len());
    for (i, role) in template.roles.iter().enumerate() {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO roles (id, community_id, name, permissions, color, position) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(community_id)
        .bind(&role.name)
        .bind(role.permissions.0)
        .bind(role.color)
        .bind(i32::try_from(i + 1).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
        role_ids.push(id);
    }

    let mut category_ids = Vec::with_capacity(template.categories.len());
    for (i, category) in template.categories.iter().enumerate() {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO channel_categories (id, community_id, name, position) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(community_id)
        .bind(&category.name)
        .bind(i32::try_from(i).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
        category_ids.push(id);
    }

    let mut channel_ids = Vec::with_capacity(template.channels.len());
    for channel in &template.channels {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, kind, topic, position, category_id, audio_bitrate, user_limit)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(community_id)
        .bind(&channel.name)
        .bind(channel.kind)
        .bind(&channel.topic)
        .bind(channel.position)
        .bind(channel.category.map(|i| category_ids[i]))
        .bind(channel.audio_bitrate)
        .bind(channel.user_limit)
        .execute(&mut *tx)
        .await?;
        for o in &channel.overrides {
            let role_id = match o.role {
                RoleRef::Default => default_role_id,
                RoleRef::Role(i) => role_ids[i],
            };
            sqlx::query(
                "INSERT INTO channel_overrides (channel_id, role_id, allow, deny) VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(role_id)
            .bind(o.allow.0)
            .bind(o.deny.0)
            .execute(&mut *tx)
            .await?;
        }
        channel_ids.push(id);
    }

    // Ohne Systemkanal in der Vorlage übernimmt der erste Textkanal die Willkommensnachrichten (wie bei neuen
    // Communities ohne Vorlage).
    let system_channel =
        template.system_channel.or_else(|| template.channels.iter().position(|c| c.kind == "text"));
    if let Some(i) = system_channel {
        sqlx::query("UPDATE communities SET system_channel_id = $2 WHERE id = $1")
            .bind(community_id)
            .bind(channel_ids[i])
            .execute(&mut *tx)
            .await?;
    }
    if let Some(i) = template.afk_channel {
        sqlx::query(
            "UPDATE communities SET afk_channel_id = $2, afk_timeout_seconds = COALESCE($3, afk_timeout_seconds)
             WHERE id = $1",
        )
        .bind(community_id)
        .bind(channel_ids[i])
        .bind(template.afk_timeout)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_from_links_and_text() {
        for input in [
            "HqjFNRCT6EEU",
            "  HqjFNRCT6EEU  ",
            "https://discord.new/HqjFNRCT6EEU",
            "discord.new/HqjFNRCT6EEU",
            "https://discord.com/template/HqjFNRCT6EEU",
            "https://www.discord.com/template/HqjFNRCT6EEU?ref=x#y",
            "https://discordapp.com/template/HqjFNRCT6EEU",
            "https://discord.gg/template/HqjFNRCT6EEU",
            "http://DISCORD.NEW/HqjFNRCT6EEU/",
        ] {
            assert_eq!(extract_code(input).as_deref(), Some("HqjFNRCT6EEU"), "{input}");
        }
    }

    #[test]
    fn foreign_hosts_and_junk_are_rejected() {
        for input in [
            "",
            "a",
            "https://evil.example/template/HqjFNRCT6EEU",
            "https://discord.new.evil.example/HqjFNRCT6EEU",
            "https://evil.example/discord.new/HqjFNRCT6EEU",
            "https://discord.new@evil.example/HqjFNRCT6EEU",
            "https://user@discord.new/HqjFNRCT6EEU",
            "https://discord.new:8443/HqjFNRCT6EEU",
            "https://discord.com/invite/HqjFNRCT6EEU",
            "https://discord.com/HqjFNRCT6EEU",
            "https://discord.new/a/b",
            "https://discord.new/HqjF%2FNRCT",
            "ftp://discord.new/HqjFNRCT6EEU",
            "file:///etc/passwd",
            "Hqj FNRCT",
            "../../admin",
            &"x".repeat(65),
        ] {
            assert_eq!(extract_code(input), None, "{input}");
        }
    }

    #[test]
    fn clip_and_names() {
        assert_eq!(clip("  a\u{0}b  ", 10), "ab");
        assert_eq!(clip("abcdef", 3), "abc");
        assert_eq!(channel_name("  Off Topic ", "text"), "off-topic");
        assert_eq!(channel_name("Lounge Raum", "voice"), "Lounge Raum");
        assert_eq!(channel_name("   ", "text"), "kanal");
        assert_eq!(channel_name(&"x".repeat(100), "voice").chars().count(), 64);
        let long = channel_name(&"Ab ".repeat(40), "text");
        assert_eq!(long.chars().count(), 64);
        assert_eq!(channels::normalize_channel_name(&long, "text").unwrap(), long);
    }

    fn minimal(roles: &str, channels: &str) -> Vec<u8> {
        format!(
            r#"{{"code":"abc","name":"Vorlage","description":null,"serialized_source_guild":{{
                "name":"Server","description":"Beschreibung","afk_channel_id":3,"afk_timeout":600,
                "system_channel_id":2,"roles":[{roles}],"channels":[{channels}]}}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn roles_are_reversed_and_everyone_becomes_default() {
        let t = parse(&minimal(
            r#"{"id":0,"name":"@everyone","permissions":"1024","color":0},
               {"id":1,"name":"Unten","permissions":"0","color":255,"hoist":true},
               {"id":2,"name":"Oben","permissions":"8","color":0}"#,
            "",
        ))
        .unwrap();
        assert_eq!(t.default_permissions, Permissions::VIEW_CHANNEL);
        assert_eq!(t.roles.len(), 2);
        assert_eq!(t.roles[0].name, "Oben");
        assert_eq!(t.roles[0].permissions, Permissions::ADMINISTRATOR);
        assert_eq!(t.roles[0].color, None);
        assert_eq!(t.roles[1].name, "Unten");
        assert_eq!(t.roles[1].color, Some(255));
        assert!(t.notes.iter().any(|n| n.contains("erwähnbar")));
        assert_eq!(t.name, "Vorlage");
        assert_eq!(t.description.as_deref(), Some("Beschreibung"));
    }

    #[test]
    fn invalid_or_oversized_templates_fail() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"nicht json").is_err());
        assert!(parse(&minimal(r#"{"id":0,"name":"x","permissions":"keine-zahl"}"#, "")).is_err());
        let roles: Vec<String> =
            (0..=MAX_ROLES).map(|i| format!(r#"{{"id":{i},"name":"r","permissions":"0"}}"#)).collect();
        assert!(parse(&minimal(&roles.join(","), "")).is_err());
    }

    #[test]
    fn voice_limits_are_clamped_and_afk_parsed() {
        let t = parse(&minimal(
            r#"{"id":0,"name":"@everyone","permissions":"0"}"#,
            r#"{"id":2,"type":0,"name":"Allgemein Chat","position":0},
               {"id":3,"type":2,"name":"AFK","position":0,"bitrate":384000,"user_limit":150},
               {"id":4,"type":13,"name":"Bühne","position":1,"bitrate":8000,"user_limit":0},
               {"id":5,"type":99,"name":"Unbekannt","position":0}"#,
        ))
        .unwrap();
        assert_eq!(t.channels.len(), 3);
        assert_eq!(t.channels[0].name, "allgemein-chat");
        assert_eq!(t.channels[1].audio_bitrate, 256);
        assert_eq!(t.channels[1].user_limit, Some(99));
        assert_eq!(t.channels[2].audio_bitrate, 32);
        assert_eq!(t.channels[2].user_limit, None);
        assert_eq!(t.system_channel, Some(0));
        assert_eq!(t.afk_channel, Some(1));
        assert_eq!(t.afk_timeout, Some(900));
        assert!(t.notes.iter().any(|n| n.contains("übersprungen")));
    }
}
