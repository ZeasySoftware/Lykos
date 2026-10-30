//! Discord-kompatible Embeds (Karten in Nachrichten) und `allowed_mentions` für Webhooks und Bots.
//!
//! Eingehende Embeds werden großzügig gelesen (unbekannte Felder und falsche Typen werden ignoriert, zu
//! lange Texte gekürzt), damit bestehende Integrationen ohne Anpassung funktionieren. Gespeichert wird nur
//! eine bereinigte Form: Titel, Beschreibung, Link, Farbe, Zeitstempel, Felder, Autor-Name/-Link und
//! Fußzeilen-Text. **Bilder (`image`, `thumbnail`, `author.icon_url`, `footer.icon_url`, `video`) werden
//! bewusst verworfen** – Clients würden sie sonst direkt von fremden Servern laden und dabei die IP-Adressen
//! der Lesenden preisgeben.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// Höchstzahl Embeds je Nachricht.
pub const MAX_EMBEDS: usize = 10;
/// Höchstzahl Felder je Embed.
pub const MAX_FIELDS: usize = 25;
/// Summe aller Texte aller Embeds einer Nachricht (wie Discord).
pub const MAX_TOTAL_CHARS: usize = 6000;
const TITLE_MAX: usize = 256;
const DESCRIPTION_MAX: usize = 4096;
const FIELD_NAME_MAX: usize = 256;
const FIELD_VALUE_MAX: usize = 1024;
const FOOTER_MAX: usize = 2048;
const AUTHOR_MAX: usize = 256;
const URL_MAX: usize = 2048;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embed {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Link des Titels (nur `http`/`https`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Farbe des Randstreifens (0xRRGGBB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<i32>,
    /// Zeitpunkt (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<EmbedField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedFooter {
    pub text: String,
}

impl Embed {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.fields.is_empty()
            && self.author.is_none()
            && self.footer.is_none()
    }

    /// Zeichen aller Texte (für `MAX_TOTAL_CHARS`).
    fn chars(&self) -> usize {
        let len = |s: &Option<String>| s.as_deref().map_or(0, |s| s.chars().count());
        len(&self.title)
            + len(&self.description)
            + self.fields.iter().map(|f| f.name.chars().count() + f.value.chars().count()).sum::<usize>()
            + self.author.as_ref().map_or(0, |a| a.name.chars().count())
            + self.footer.as_ref().map_or(0, |f| f.text.chars().count())
    }
}

/// Text bereinigen: Steuerzeichen außer Zeilenumbruch/Tab entfernen, trimmen, auf `max` Zeichen kürzen;
/// leer → `None`.
fn text(v: Option<&Value>, max: usize) -> Option<String> {
    let raw = match v? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let cleaned: String = raw.chars().filter(|c| matches!(c, '\n' | '\t') || !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out: String = trimmed.chars().take(max).collect();
    if trimmed.chars().count() > max {
        out.pop();
        out.push('…');
    }
    Some(out)
}

/// Nur absolute `http(s)`-Links ohne Leer- und Steuerzeichen.
fn url(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    let lower = s.to_ascii_lowercase();
    let ok = (lower.starts_with("https://") || lower.starts_with("http://"))
        && s.len() <= URL_MAX
        && s.len() > "https://".len()
        && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| s.to_owned())
}

fn color(v: Option<&Value>) -> Option<i32> {
    let n = match v? {
        Value::Number(n) => n.as_i64()?,
        Value::String(s) => s.trim().trim_start_matches('#').parse::<i64>().ok()?,
        _ => return None,
    };
    (0..=0xFF_FFFF).contains(&n).then_some(n as i32)
}

fn timestamp(v: Option<&Value>) -> Option<String> {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let parsed = OffsetDateTime::parse(v?.as_str()?.trim(), &Rfc3339).ok()?;
    parsed.format(&Rfc3339).ok()
}

fn embed(v: &Value) -> Option<Embed> {
    let obj = v.as_object()?;
    let fields = obj
        .get("fields")
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .filter_map(|f| {
                    let name = text(f.get("name"), FIELD_NAME_MAX);
                    let value = text(f.get("value"), FIELD_VALUE_MAX);
                    if name.is_none() && value.is_none() {
                        return None;
                    }
                    Some(EmbedField {
                        name: name.unwrap_or_default(),
                        value: value.unwrap_or_default(),
                        inline: f.get("inline").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .take(MAX_FIELDS)
                .collect()
        })
        .unwrap_or_default();
    let author = obj
        .get("author")
        .and_then(|a| Some(EmbedAuthor { name: text(a.get("name"), AUTHOR_MAX)?, url: url(a.get("url")) }));
    let footer = obj.get("footer").and_then(|f| Some(EmbedFooter { text: text(f.get("text"), FOOTER_MAX)? }));
    let embed = Embed {
        title: text(obj.get("title"), TITLE_MAX),
        description: text(obj.get("description"), DESCRIPTION_MAX),
        url: url(obj.get("url")),
        color: color(obj.get("color")),
        timestamp: timestamp(obj.get("timestamp")),
        fields,
        author,
        footer,
    };
    (!embed.is_empty()).then_some(embed)
}

/// Liest `embeds` (Discord-Format) großzügig ein. Mehr als `MAX_EMBEDS` oder zusammen mehr als
/// `MAX_TOTAL_CHARS` Zeichen → `400 invalid_embeds`. Leere Embeds (z. B. nur ein Bild) fallen weg.
pub fn parse(v: Option<&Value>) -> ApiResult<Vec<Embed>> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let list = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(list) => list,
        _ => return Err(ApiError::BadRequest("invalid_embeds")),
    };
    if list.len() > MAX_EMBEDS {
        return Err(ApiError::BadRequest("invalid_embeds"));
    }
    let embeds: Vec<Embed> = list.iter().filter_map(embed).collect();
    if embeds.iter().map(Embed::chars).sum::<usize>() > MAX_TOTAL_CHARS {
        return Err(ApiError::BadRequest("invalid_embeds"));
    }
    Ok(embeds)
}

/// Wen eine Nachricht erwähnen darf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMentions {
    /// Alle per `@name` erwähnten Mitglieder.
    All,
    /// Nur diese Nutzer (sofern im Text erwähnt).
    Only(Vec<Uuid>),
    /// Niemanden.
    None,
}

/// Erwähnungs-Regeln einer Nachricht. Für Nutzer und Bots: alle Nutzer, `@everyone` nur mit
/// `MENTION_EVERYONE`. Für Webhooks aus `allowed_mentions` (siehe `allowed_mentions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionPolicy {
    pub users: UserMentions,
    pub everyone: bool,
}

impl MentionPolicy {
    pub fn user_ids(&self) -> Option<Vec<Uuid>> {
        match &self.users {
            UserMentions::All => None,
            UserMentions::Only(ids) => Some(ids.clone()),
            UserMentions::None => Some(Vec::new()),
        }
    }
}

/// `allowed_mentions` eines Webhooks. Ohne Angabe: Nutzer-Erwähnungen ja, `@everyone` nein (sicherer als
/// Discords Standard). Mit Angabe wie bei Discord: `parse` mit `"users"` / `"everyone"` (`"roles"` wird
/// ignoriert, Lykos hat keine Rollen-Erwähnungen), `users` = Liste erlaubter Nutzer-IDs.
pub fn allowed_mentions(v: Option<&Value>) -> MentionPolicy {
    let Some(obj) = v.and_then(Value::as_object) else {
        return MentionPolicy { users: UserMentions::All, everyone: false };
    };
    let parse: Vec<&str> = obj
        .get("parse")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |p| p.iter().filter_map(Value::as_str).collect());
    let ids: Vec<Uuid> = obj
        .get("users")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |u| u.iter().filter_map(|id| id.as_str()?.parse().ok()).take(100).collect());
    let users = if parse.contains(&"users") {
        UserMentions::All
    } else if !ids.is_empty() {
        UserMentions::Only(ids)
    } else {
        UserMentions::None
    };
    MentionPolicy { users, everyone: parse.contains(&"everyone") }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn discord_example_is_cleaned() {
        let embeds = parse(Some(&json!([{
            "title": "  Neuer Push  ",
            "description": "3 Commits\u{0} nach main",
            "url": "https://git.example.org/x",
            "color": 5814783,
            "timestamp": "2026-10-30T12:00:00.000Z",
            "fields": [{ "name": "Branch", "value": "main", "inline": true }, { "name": "", "value": "" }],
            "author": { "name": "alex", "url": "javascript:alert(1)", "icon_url": "https://tracker.example/a.png" },
            "footer": { "text": "Gitea", "icon_url": "https://tracker.example/f.png" },
            "image": { "url": "https://tracker.example/i.png" },
            "thumbnail": { "url": "https://tracker.example/t.png" }
        }])))
        .unwrap();
        assert_eq!(embeds.len(), 1);
        let e = &embeds[0];
        assert_eq!(e.title.as_deref(), Some("Neuer Push"));
        assert_eq!(e.description.as_deref(), Some("3 Commits nach main"));
        assert_eq!(e.url.as_deref(), Some("https://git.example.org/x"));
        assert_eq!(e.color, Some(5814783));
        assert_eq!(e.timestamp.as_deref(), Some("2026-10-30T12:00:00Z"));
        assert_eq!(e.fields, vec![EmbedField { name: "Branch".into(), value: "main".into(), inline: true }]);
        assert_eq!(e.author, Some(EmbedAuthor { name: "alex".into(), url: None }));
        assert_eq!(e.footer, Some(EmbedFooter { text: "Gitea".into() }));
        // Keine Bild-URL überlebt die Bereinigung.
        assert!(!serde_json::to_string(&embeds).unwrap().contains("tracker.example"));
    }

    #[test]
    fn limits() {
        assert_eq!(parse(None).unwrap(), vec![]);
        assert_eq!(parse(Some(&json!(null))).unwrap(), vec![]);
        assert!(parse(Some(&json!("x"))).is_err());
        assert!(parse(Some(&json!(vec![json!({ "title": "x" }); 11]))).is_err());
        // Nur ein Bild → leer → fällt weg.
        assert_eq!(parse(Some(&json!([{ "image": { "url": "https://x/y.png" } }]))).unwrap(), vec![]);
        // Zu lange Texte werden gekürzt.
        let long = parse(Some(&json!([{ "title": "a".repeat(300) }]))).unwrap();
        assert_eq!(long[0].title.as_ref().unwrap().chars().count(), TITLE_MAX);
        // Summe > 6000 Zeichen → abgelehnt.
        let big = json!([{ "description": "a".repeat(4000) }, { "description": "b".repeat(4000) }]);
        assert!(parse(Some(&big)).is_err());
        // Farbe außerhalb von 24 Bit fällt weg.
        assert_eq!(parse(Some(&json!([{ "title": "x", "color": -1 }]))).unwrap()[0].color, None);
    }

    #[test]
    fn mention_rules() {
        let def = allowed_mentions(None);
        assert_eq!(def, MentionPolicy { users: UserMentions::All, everyone: false });
        let none = allowed_mentions(Some(&json!({ "parse": [] })));
        assert_eq!(none, MentionPolicy { users: UserMentions::None, everyone: false });
        let all = allowed_mentions(Some(&json!({ "parse": ["everyone", "users", "roles"] })));
        assert_eq!(all, MentionPolicy { users: UserMentions::All, everyone: true });
        let id = Uuid::now_v7();
        let only = allowed_mentions(Some(&json!({ "users": [id.to_string(), "kaputt"] })));
        assert_eq!(only.users, UserMentions::Only(vec![id]));
        assert_eq!(only.user_ids(), Some(vec![id]));
    }
}
