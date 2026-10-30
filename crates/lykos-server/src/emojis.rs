//! Custom-Emotes: Datenobjekt, Abfragen und die Emote-Tokens im Nachrichtentext.
//!
//! Format im Text wie bei Discord: `<:name:id>` (statisch) bzw. `<a:name:id>` (animiert). Beim Senden und
//! Bearbeiten prüft der Server jedes Token: Das Emote muss existieren und der Absender Mitglied der
//! besitzenden Community sein – sonst wird das Token zu `:name:` (Text). Gültige Tokens werden auf den
//! aktuellen Namen und das richtige Präfix (`a` nur bei animierten) normalisiert.
//! Tokens in Code (```Block``` oder `inline`) bleiben unverändert – der Client zeigt sie dort als Text.
//!
//! Reaktionen mit Custom-Emotes nutzen `name:id` (ohne spitze Klammern); gespeichert wird nur die ID.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::ApiResult, models::UserPublic};

/// Höchstzahl an Emotes pro Community.
pub const MAX_PER_COMMUNITY: i64 = 100;
/// Höchstgröße einer Emote-Datei.
pub const MAX_BYTES: usize = 256 * 1024;
/// Größte zulässige Kantenlänge in Pixeln.
pub const MAX_SIDE: i32 = 128;
/// Erlaubte Bildtypen (Magic Bytes). JPEG bewusst nicht: Emotes brauchen Transparenz.
pub const ALLOWED_TYPES: &[&str] = &["image/png", "image/gif", "image/webp"];

/// Name: 2–32 Zeichen `[A-Za-z0-9_]`.
pub fn is_valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Dateiendung zum Inhaltstyp (für die URL; der Server liefert immer den gespeicherten Typ).
pub fn extension(content_type: &str) -> &'static str {
    match content_type {
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// Abruf-Pfad eines Emotes (relativ zur Instanz). Die ID ändert sich nie → unbegrenzt cachebar.
pub fn url(id: Uuid, content_type: &str) -> String {
    format!("/api/emojis/{id}.{}", extension(content_type))
}

// --- Tokens im Text -----------------------------------------------------------

/// Ein Emote-Token `<:name:id>` / `<a:name:id>` im Text (Byte-Positionen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub animated: bool,
    pub name: String,
    pub id: Uuid,
}

/// Byte-Bereiche von Code (```Block``` und `inline` innerhalb einer Zeile), jeweils nur mit schließendem
/// Begrenzer – wie `mentions::strip_code` und der Client-Parser.
fn code_ranges(content: &str) -> Vec<(usize, usize)> {
    let b = content.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"```")
            && let Some(rel) = content[i + 3..].find("```")
        {
            let end = i + 3 + rel + 3;
            out.push((i, end));
            i = end;
            continue;
        }
        if b[i] == b'`'
            && let Some(rel) = content[i + 1..].find(['`', '\n'])
            && b[i + 1 + rel] == b'`'
            && rel > 0
        {
            let end = i + 1 + rel + 1;
            out.push((i, end));
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// Versucht ab `i` (ein `<`) ein Token zu lesen.
fn token_at(content: &str, i: usize) -> Option<Token> {
    let b = content.as_bytes();
    let mut j = i + 1;
    let animated = b.get(j) == Some(&b'a');
    if animated {
        j += 1;
    }
    if b.get(j) != Some(&b':') {
        return None;
    }
    j += 1;
    let name_start = j;
    while j < b.len() && j - name_start <= 32 && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
    }
    let name = &content[name_start..j];
    if !is_valid_name(name) || b.get(j) != Some(&b':') {
        return None;
    }
    j += 1;
    let id_str = content.get(j..j + 36)?;
    if b.get(j + 36) != Some(&b'>') {
        return None;
    }
    let id = Uuid::try_parse(id_str).ok()?;
    Some(Token { start: i, end: j + 37, animated, name: name.to_owned(), id })
}

/// Alle Emote-Tokens außerhalb von Code, in Textreihenfolge.
pub fn tokens(content: &str) -> Vec<Token> {
    let code = code_ranges(content);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = content[i..].find('<') {
        let at = i + rel;
        if let Some(&(_, end)) = code.iter().find(|(s, e)| (*s..*e).contains(&at)) {
            i = end;
            continue;
        }
        match token_at(content, at) {
            Some(t) => {
                i = t.end;
                out.push(t);
            }
            None => i = at + 1,
        }
    }
    out
}

/// Ersetzt jedes Token durch `replace(token)`.
pub fn rewrite(content: &str, mut replace: impl FnMut(&Token) -> String) -> String {
    let mut out = String::with_capacity(content.len());
    let mut last = 0;
    for t in tokens(content) {
        out.push_str(&content[last..t.start]);
        out.push_str(&replace(&t));
        last = t.end;
    }
    out.push_str(&content[last..]);
    out
}

/// Kanonische Schreibweise eines Tokens.
pub fn format_token(name: &str, id: Uuid, animated: bool) -> String {
    format!("<{}:{name}:{id}>", if animated { "a" } else { "" })
}

/// Reaktion `name:id` mit Custom-Emote → `(name, id)`; sonst `None` (Unicode-Emoji).
pub fn parse_reaction(emoji: &str) -> Option<(&str, Uuid)> {
    let (name, id) = emoji.split_once(':')?;
    if !is_valid_name(name) || id.len() != 36 {
        return None;
    }
    Some((name, Uuid::try_parse(id).ok()?))
}

// --- Datenobjekt und Abfragen -------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CustomEmoji {
    pub id: Uuid,
    pub community_id: Uuid,
    pub name: String,
    pub animated: bool,
    /// Abruf-Pfad, z. B. `/api/emojis/{id}.png`.
    pub url: String,
    /// Wer das Emote hochgeladen hat (`null`, wenn das Konto gelöscht ist).
    pub uploader: Option<UserPublic>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    community_id: Uuid,
    name: String,
    animated: bool,
    content_type: String,
    created_at: OffsetDateTime,
    uploader_id: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
}

impl From<Row> for CustomEmoji {
    fn from(r: Row) -> Self {
        CustomEmoji {
            url: url(r.id, &r.content_type),
            id: r.id,
            community_id: r.community_id,
            name: r.name,
            animated: r.animated,
            uploader: match (r.uploader_id, r.username) {
                (Some(id), Some(username)) => {
                    Some(UserPublic::new(id, username, r.display_name, r.avatar_id))
                }
                _ => None,
            },
            created_at: r.created_at,
        }
    }
}

macro_rules! emoji_select {
    () => {
        "SELECT e.id, e.community_id, e.name, e.animated, e.content_type, e.created_at, e.uploader_id,
                u.username, u.display_name, u.avatar_id
         FROM custom_emojis e LEFT JOIN users u ON u.id = e.uploader_id"
    };
}

/// Alle Emotes einer Community, alphabetisch.
pub async fn of_community(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Vec<CustomEmoji>> {
    let rows: Vec<Row> =
        sqlx::query_as(concat!(emoji_select!(), " WHERE e.community_id = $1 ORDER BY lower(e.name), e.id"))
            .bind(community_id)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(CustomEmoji::from).collect())
}

/// Alle Emotes aller Communities, in denen `user_id` Mitglied ist (= die er überall verwenden darf).
pub async fn usable_by(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<CustomEmoji>> {
    let rows: Vec<Row> = sqlx::query_as(concat!(
        emoji_select!(),
        " JOIN members m ON m.community_id = e.community_id AND m.user_id = $1
         ORDER BY e.community_id, lower(e.name), e.id"
    ))
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(CustomEmoji::from).collect())
}

pub async fn by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<CustomEmoji>> {
    let row: Option<Row> =
        sqlx::query_as(concat!(emoji_select!(), " WHERE e.id = $1")).bind(id).fetch_optional(db).await?;
    Ok(row.map(CustomEmoji::from))
}

/// Prüft alle Emote-Tokens im Text gegen die Mitgliedschaften des Absenders (siehe Moduldoku).
pub async fn sanitize(db: impl PgExecutor<'_>, user_id: Uuid, content: &str) -> ApiResult<String> {
    let found = tokens(content);
    if found.is_empty() {
        return Ok(content.to_owned());
    }
    let mut ids: Vec<Uuid> = found.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let usable: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT e.id, e.name, e.animated FROM custom_emojis e
         JOIN members m ON m.community_id = e.community_id AND m.user_id = $1
         WHERE e.id = ANY($2)",
    )
    .bind(user_id)
    .bind(&ids)
    .fetch_all(db)
    .await?;
    let usable: HashMap<Uuid, (String, bool)> =
        usable.into_iter().map(|(id, name, animated)| (id, (name, animated))).collect();
    Ok(rewrite(content, |t| match usable.get(&t.id) {
        Some((name, animated)) => format_token(name, t.id, *animated),
        None => format!(":{}:", t.name),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0192f3a0-1111-7000-8000-000000000001";

    #[test]
    fn names() {
        for ok in ["ok", "wolf_heult", "A1", &"x".repeat(32)] {
            assert!(is_valid_name(ok), "{ok}");
        }
        for bad in ["", "a", "mit leer", "ümlaut", "a-b", "a:b", &"x".repeat(33)] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn finds_tokens() {
        let text = format!("Hi <:wolf:{ID}> und <a:tanz_1:{ID}>!");
        let t = tokens(&text);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].name.as_str(), t[0].animated), ("wolf", false));
        assert_eq!((t[1].name.as_str(), t[1].animated), ("tanz_1", true));
        assert_eq!(&text[t[0].start..t[0].end], format!("<:wolf:{ID}>"));
        assert_eq!(t[0].id, Uuid::parse_str(ID).unwrap());
    }

    #[test]
    fn ignores_invalid_and_code() {
        for text in [
            format!("<:w:{ID}>"),
            format!("<:wolf:{ID}"),
            format!("<b:wolf:{ID}>"),
            "<:wolf:nicht-eine-uuid-aber-36-zeichen-lang>".to_owned(),
            format!("`<:wolf:{ID}>`"),
            format!("```\n<:wolf:{ID}>\n```"),
        ] {
            assert!(tokens(&text).is_empty(), "{text}");
        }
        // Offener Backtick ist kein Code.
        assert_eq!(tokens(&format!("` <:wolf:{ID}>")).len(), 1);
    }

    #[test]
    fn rewrites_tokens() {
        let text = format!("ä <:wolf:{ID}> ö `<:wolf:{ID}>` <:x_y:{ID}>");
        let out = rewrite(&text, |t| format!(":{}:", t.name));
        assert_eq!(out, format!("ä :wolf: ö `<:wolf:{ID}>` :x_y:"));
        assert_eq!(rewrite("ohne", |_| unreachable!()), "ohne");
    }

    #[test]
    fn reaction_keys() {
        let key = format!("wolf:{ID}");
        let (name, id) = parse_reaction(&key).unwrap();
        assert_eq!((name, id.to_string()), ("wolf", ID.to_owned()));
        assert!(parse_reaction("👍").is_none());
        assert!(parse_reaction(&format!("w:{ID}")).is_none());
        assert!(parse_reaction("wolf:123").is_none());
        assert_eq!(format_token("wolf", id, true), format!("<a:wolf:{ID}>"));
    }
}
