//! Dateianhänge: Datenobjekt, Abfragen, Prüfung hochgeladener Dateien und das Aufräumen
//! (unverknüpfte Uploads, Aufbewahrungsfrist, verwaiste Dateien im Speicher).
//! Avatare, Profilbanner, Community-Icons und -Banner sowie Feedback-Screenshots liegen im selben
//! Speicher (Schlüssel = ihre ID) und zählen beim Abgleich als bekannt.

use std::{collections::HashSet, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::ApiResult, storage::Storage};

/// Höchstzahl an Anhängen pro Nachricht.
pub const MAX_PER_MESSAGE: usize = 10;
/// Maximale Länge eines Dateinamens (Zeichen).
pub const MAX_FILENAME_CHARS: usize = 200;
/// So viele Bytes vom Dateianfang reichen für Typ-Erkennung und Bildmaße.
pub const SNIFF_BYTES: usize = 64 * 1024;
/// Unverknüpfte Uploads werden nach dieser Zeit gelöscht.
const UNLINKED_TTL_HOURS: i32 = 24;
/// Dateien ohne DB-Eintrag gelten erst nach dieser Zeit als verwaist (laufende Uploads schützen).
pub const ORPHAN_GRACE: Duration = Duration::from_secs(60 * 60);
const INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Bildtypen, die inline angezeigt werden dürfen (keine SVG – die kann Skripte enthalten).
pub const INLINE_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Attachment {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub message_id: Option<Uuid>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Download-Pfad relativ zur Instanz, z. B. `/api/attachments/{id}/{filename}`.
    #[serde(default)]
    #[sqlx(skip)]
    pub url: String,
}

impl Attachment {
    pub fn with_url(mut self) -> Self {
        self.url = format!("/api/attachments/{}/{}", self.id, percent_encode(&self.filename));
        self
    }
}

/// Spalten für `json_build_object` – in `models::message_select!` als Unterabfrage genutzt.
#[macro_export]
macro_rules! attachment_json {
    () => {
        "COALESCE((SELECT json_agg(json_build_object(
            'id', a.id, 'channel_id', a.channel_id, 'message_id', a.message_id,
            'filename', a.filename, 'content_type', a.content_type, 'size_bytes', a.size_bytes,
            'width', a.width, 'height', a.height, 'created_at', a.created_at) ORDER BY a.id)
          FROM attachments a WHERE a.message_id = msg.id), '[]'::json)"
    };
}

macro_rules! attachment_columns {
    () => {
        "id, channel_id, message_id, filename, content_type, size_bytes, width, height, created_at"
    };
}

/// Download-Recht-Prüfung braucht zusätzlich Uploader und Community.
#[derive(Debug, Clone, FromRow)]
pub struct AttachmentAccess {
    #[sqlx(flatten)]
    pub attachment: Attachment,
    pub uploader_id: Option<Uuid>,
}

pub async fn by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<AttachmentAccess>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        attachment_columns!(),
        ", uploader_id FROM attachments WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?
    .map(|mut a: AttachmentAccess| {
        a.attachment = a.attachment.with_url();
        a
    }))
}

pub struct NewAttachment<'a> {
    pub id: Uuid,
    /// `None` in DM-Kanälen.
    pub community_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub uploader_id: Uuid,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub dimensions: Option<(i32, i32)>,
}

pub async fn insert(db: impl PgExecutor<'_>, new: NewAttachment<'_>) -> ApiResult<Attachment> {
    let a: Attachment = sqlx::query_as(concat!(
        "INSERT INTO attachments (id, community_id, channel_id, uploader_id, filename, content_type,
                                  size_bytes, width, height)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING ",
        attachment_columns!()
    ))
    .bind(new.id)
    .bind(new.community_id)
    .bind(new.channel_id)
    .bind(new.uploader_id)
    .bind(new.filename)
    .bind(new.content_type)
    .bind(new.size_bytes)
    .bind(new.dimensions.map(|d| d.0))
    .bind(new.dimensions.map(|d| d.1))
    .fetch_one(db)
    .await?;
    Ok(a.with_url())
}

/// Löscht Dateien im Speicher, Fehler werden nur protokolliert (der Abgleich im Job holt sie nach).
pub async fn delete_files(storage: &dyn Storage, ids: &[Uuid]) {
    for id in ids {
        if let Err(e) = storage.delete(&id.to_string()).await {
            tracing::warn!(error = %e, %id, "Anhang-Datei konnte nicht gelöscht werden");
        }
    }
}

// --- Prüfung von Uploads ------------------------------------------------------

/// Dateiname für Anzeige und `Content-Disposition`: nur der letzte Pfadteil, ohne Steuerzeichen,
/// ohne führende/abschließende Punkte und Leerzeichen, höchstens `MAX_FILENAME_CHARS` Zeichen.
pub fn sanitize_filename(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or_default();
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'))
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.');
    let mut name: String = trimmed.chars().take(MAX_FILENAME_CHARS).collect();
    if name.is_empty() {
        name = "datei".into();
    }
    name
}

/// Inhaltstyp aus den ersten Bytes (Magic Bytes) – die Angabe des Clients zählt nicht.
pub fn detect_content_type(head: &[u8]) -> &'static str {
    infer::get(head).map(|t| t.mime_type()).unwrap_or("application/octet-stream")
}

/// Bildmaße aus dem Dateikopf, nur für inline anzeigbare Bildtypen.
pub fn image_dimensions(content_type: &str, head: &[u8]) -> Option<(i32, i32)> {
    if !INLINE_IMAGE_TYPES.contains(&content_type) {
        return None;
    }
    let size = imagesize::blob_size(head).ok()?;
    Some((i32::try_from(size.width).ok()?, i32::try_from(size.height).ok()?))
}

/// Typen, die ein Browser ausführen oder rendern könnte, werden nur als Bytes ausgeliefert.
pub fn served_content_type(content_type: &str) -> &str {
    let active = content_type.starts_with("text/")
        || content_type.contains("html")
        || content_type.contains("xml")
        || content_type.contains("svg")
        || content_type.contains("javascript");
    if active { "application/octet-stream" } else { content_type }
}

/// Prozent-Kodierung für Pfadsegmente und `filename*` (RFC 3986 „unreserved“ bleibt stehen).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `Content-Disposition` mit ASCII-Fallback und UTF-8-Namen (RFC 6266/5987).
pub fn content_disposition(inline: bool, filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' { c } else { '_' })
        .collect();
    let kind = if inline { "inline" } else { "attachment" };
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{}", percent_encode(filename))
}

// --- Aufräumen ----------------------------------------------------------------

pub fn spawn_cleanup(db: PgPool, storage: Arc<dyn Storage>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(INTERVAL);
        loop {
            tick.tick().await;
            if let Err(e) = cleanup(&db, storage.as_ref(), ORPHAN_GRACE).await {
                tracing::warn!(error = %e, "Aufräumen der Anhänge fehlgeschlagen");
            }
        }
    });
}

/// Löscht unverknüpfte Uploads (> 24 h), Anhänge abgelaufener Nachrichten (Aufbewahrungsfrist der
/// Community) und Dateien ohne DB-Eintrag, die älter als `orphan_grace` sind (z. B. nach dem
/// Löschen von Kanälen/Communities oder Nachrichten durch den Aufbewahrungs-Job).
/// Mehrfaches, paralleles Laufen ist unschädlich.
pub async fn cleanup(db: &PgPool, storage: &dyn Storage, orphan_grace: Duration) -> anyhow::Result<()> {
    let unlinked: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM attachments WHERE message_id IS NULL AND created_at < now() - make_interval(hours => $1)
         RETURNING id",
    )
    .bind(UNLINKED_TTL_HOURS)
    .fetch_all(db)
    .await?;

    let expired: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM attachments a USING messages m, communities co
         WHERE a.message_id = m.id AND a.community_id = co.id
           AND co.message_retention_days IS NOT NULL
           AND m.created_at < now() - make_interval(days => co.message_retention_days)
         RETURNING a.id",
    )
    .fetch_all(db)
    .await?;

    delete_files(storage, &unlinked).await;
    delete_files(storage, &expired).await;

    // Abgleich: Dateien ohne Zeile in `attachments` und ohne Verweis als Avatar, Profilbanner,
    // Community-Icon oder -Banner, Feedback-Screenshot, Custom-Emote oder Webhook-Avatar.
    let known: HashSet<Uuid> = sqlx::query_scalar(
        "SELECT id FROM attachments
         UNION ALL SELECT avatar_id FROM users WHERE avatar_id IS NOT NULL
         UNION ALL SELECT banner_id FROM users WHERE banner_id IS NOT NULL
         UNION ALL SELECT icon_id FROM communities WHERE icon_id IS NOT NULL
         UNION ALL SELECT banner_id FROM communities WHERE banner_id IS NOT NULL
         UNION ALL SELECT screenshot_id FROM feedback WHERE screenshot_id IS NOT NULL
         UNION ALL SELECT id FROM custom_emojis
         UNION ALL SELECT avatar_id FROM webhooks WHERE avatar_id IS NOT NULL",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let now = std::time::SystemTime::now();
    let mut orphans = 0usize;
    for obj in storage.list().await? {
        let is_known = Uuid::parse_str(&obj.key).is_ok_and(|id| known.contains(&id));
        let old_enough = now.duration_since(obj.modified).unwrap_or_default() >= orphan_grace;
        if !is_known && old_enough {
            match storage.delete(&obj.key).await {
                Ok(()) => orphans += 1,
                Err(e) => tracing::warn!(error = %e, key = %obj.key, "verwaiste Datei nicht löschbar"),
            }
        }
    }
    storage.cleanup_incomplete(ORPHAN_GRACE.max(orphan_grace)).await?;

    if !unlinked.is_empty() || !expired.is_empty() || orphans > 0 {
        tracing::info!(
            unlinked = unlinked.len(),
            expired = expired.len(),
            orphans,
            "Aufbewahrung: Anhänge gelöscht"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(sanitize_filename(Some("../../etc/passwd")), "passwd");
        assert_eq!(sanitize_filename(Some("C:\\Users\\x\\bild.png")), "bild.png");
        assert_eq!(sanitize_filename(Some("a\u{0}b\nc\u{202e}gnp.exe")), "abcgnp.exe");
        assert_eq!(sanitize_filename(Some(" ..  ")), "datei");
        assert_eq!(sanitize_filename(None), "datei");
        assert_eq!(sanitize_filename(Some(&"ä".repeat(500))).chars().count(), MAX_FILENAME_CHARS);
    }

    #[test]
    fn content_types_from_magic_bytes() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x03\x08\x06\0\0\0";
        assert_eq!(detect_content_type(png), "image/png");
        assert_eq!(image_dimensions("image/png", png), Some((2, 3)));
        assert_eq!(detect_content_type(b"hallo"), "application/octet-stream");
        assert_eq!(served_content_type("text/html"), "application/octet-stream");
        assert_eq!(served_content_type("image/svg+xml"), "application/octet-stream");
        assert_eq!(served_content_type("application/pdf"), "application/pdf");
    }

    #[test]
    fn disposition_and_encoding() {
        assert_eq!(percent_encode("a b/ü.png"), "a%20b%2F%C3%BC.png");
        assert_eq!(
            content_disposition(false, "bericht \"final\".pdf"),
            "attachment; filename=\"bericht _final_.pdf\"; filename*=UTF-8''bericht%20%22final%22.pdf"
        );
    }
}
