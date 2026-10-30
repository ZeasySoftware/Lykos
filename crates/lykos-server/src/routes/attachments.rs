//! Upload und Download von Dateianhängen.

use std::io;

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, stream};
use uuid::Uuid;

use crate::{
    attachments::{self, Attachment, NewAttachment},
    auth::AuthUser,
    error::{ApiError, ApiResult},
    instance_settings, models,
    permissions::{self, Permissions},
    quota,
    state::AppState,
    storage::ByteStream,
};

/// Spielraum für Multipart-Rahmen (Boundary, Feld-Header) über der Dateigröße.
const MULTIPART_OVERHEAD: u64 = 64 * 1024;

/// Markiert im Upload-Stream das Überschreiten des Größenlimits.
#[derive(Debug)]
struct TooLarge;

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Datei zu groß")
    }
}

impl std::error::Error for TooLarge {}

fn is_too_large(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<TooLarge>())
}

/// `POST /channels/{id}/attachments` – multipart mit genau einem Feld `file`.
/// Das globale Body-Limit ist für diese Route abgeschaltet; die Größe wird hier gegen
/// das Upload-Limit (Instanz-Einstellung bzw. `LYKOS_MAX_UPLOAD_MB`) geprüft, während die Datei in den Speicher strömt.
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<Attachment>)> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::ATTACH_FILES)?;
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;
    if !models::is_text_like(&channel.kind) {
        return Err(ApiError::BadRequest("not_a_text_channel"));
    }

    // Grenze = Upload-Limit oder (kleiner) der Restplatz im Kontingent von Nutzer bzw. Community.
    let max_upload = instance_settings::load(&state.db, &state.config).await?.max_upload_bytes;
    let remaining = quota::remaining(&state, user.user_id, ctx.community_id).await?;
    if remaining == Some(0) {
        return Err(quota::exceeded());
    }
    let limit = remaining.map_or(max_upload, |r| r.min(max_upload));
    let too_large = move || {
        if limit < max_upload { quota::exceeded() } else { ApiError::PayloadTooLarge("file_too_large") }
    };
    let declared =
        headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > limit + MULTIPART_OVERHEAD) {
        return Err(too_large());
    }

    // Genau ein Feld `file`; andere Felder werden nicht gelesen, sondern abgelehnt (kein Puffern).
    let mut field = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("file") => f,
        Ok(_) => return Err(ApiError::BadRequest("invalid_request")),
        Err(e) => return Err(multipart_error(e)),
    };
    let filename = attachments::sanitize_filename(field.file_name());

    // Dateianfang puffern: reicht für Magic Bytes und Bildmaße.
    let mut head = Vec::new();
    while head.len() < attachments::SNIFF_BYTES {
        match field.chunk().await {
            Ok(Some(chunk)) => head.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(e) => return Err(multipart_error(e)),
        }
        if head.len() as u64 > limit {
            return Err(too_large());
        }
    }
    let content_type = attachments::detect_content_type(&head);
    let dimensions = attachments::image_dimensions(content_type, &head);

    let mut total = 0u64;
    let rest = field.map_err(io::Error::other);
    let data: ByteStream<'_> =
        Box::pin(stream::once(async { Ok(Bytes::from(head)) }).chain(rest).map(move |chunk| {
            let chunk = chunk?;
            total += chunk.len() as u64;
            if total > limit { Err(io::Error::other(TooLarge)) } else { Ok(chunk) }
        }));

    let id = Uuid::now_v7();
    let key = id.to_string();
    let size = state.storage.put(&key, data).await.map_err(|e| {
        if is_too_large(&e) {
            too_large()
        } else if e.get_ref().is_some_and(|inner| inner.is::<axum::extract::multipart::MultipartError>()) {
            ApiError::BadRequest("invalid_request")
        } else {
            ApiError::Internal(anyhow::Error::new(e).context("Anhang speichern"))
        }
    })?;

    // Erneute Prüfung beim Eintragen: parallele Uploads dürfen das Kontingent nicht gemeinsam sprengen.
    let inserted = quota::insert_within_quota(
        &state,
        NewAttachment {
            id,
            community_id: ctx.community_id,
            channel_id,
            uploader_id: user.user_id,
            filename: &filename,
            content_type,
            size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
            dimensions,
        },
    )
    .await;
    match inserted {
        Ok(attachment) => Ok((StatusCode::CREATED, Json(attachment))),
        Err(e) => {
            attachments::delete_files(state.storage.as_ref(), &[id]).await;
            Err(e)
        }
    }
}

pub(crate) fn multipart_error(e: axum::extract::multipart::MultipartError) -> ApiError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge("file_too_large")
    } else {
        ApiError::BadRequest("invalid_request")
    }
}

/// Lädt einen Anhang und prüft das Leserecht. Unverknüpfte Uploads sieht nur ihr Uploader;
/// sonst gilt `VIEW_CHANNEL` des Kanals. Alles andere ist `404` (Existenz nicht verraten).
async fn visible_attachment(state: &AppState, user: &AuthUser, id: Uuid) -> ApiResult<Attachment> {
    let access = attachments::by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?;
    if access.attachment.message_id.is_none() && access.uploader_id != Some(user.user_id) {
        return Err(ApiError::not_found());
    }
    permissions::channel_permissions(&state.db, user.user_id, access.attachment.channel_id).await?;
    Ok(access.attachment)
}

/// `GET /attachments/{id}/{filename}` – liefert die Datei (Range-Requests werden unterstützt).
/// Der Dateiname im Pfad dient nur der Lesbarkeit und wird ignoriert.
pub async fn download(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, _filename)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let attachment = visible_attachment(&state, &user, id).await?;
    let size = u64::try_from(attachment.size_bytes).unwrap_or(0);

    let range = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(value) => match parse_range(value, size) {
            Some(r) => Some(r),
            None => {
                let mut res = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                res.headers_mut().insert(header::CONTENT_RANGE, hv(&format!("bytes */{size}")));
                return Ok(res);
            }
        },
        None => None,
    };

    let stream = state
        .storage
        .get(&id.to_string(), range.clone())
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Anhang lesen")))?
        .ok_or_else(ApiError::not_found)?;

    let inline = attachments::INLINE_IMAGE_TYPES.contains(&attachment.content_type.as_str());
    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, hv(attachments::served_content_type(&attachment.content_type)));
    h.insert(
        header::CONTENT_DISPOSITION,
        hv(&attachments::content_disposition(inline, &attachment.filename)),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400"));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if !inline {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    }
    match range {
        Some(r) => {
            h.insert(header::CONTENT_LENGTH, hv(&(r.end - r.start).to_string()));
            h.insert(header::CONTENT_RANGE, hv(&format!("bytes {}-{}/{size}", r.start, r.end - 1)));
            *res.status_mut() = StatusCode::PARTIAL_CONTENT;
        }
        None => {
            h.insert(header::CONTENT_LENGTH, hv(&size.to_string()));
        }
    }
    Ok(res)
}

/// `DELETE /attachments/{id}` – der Uploader verwirft einen noch nicht gesendeten Upload.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let deleted =
        sqlx::query("DELETE FROM attachments WHERE id = $1 AND uploader_id = $2 AND message_id IS NULL")
            .bind(id)
            .bind(user.user_id)
            .execute(&state.db)
            .await?
            .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    attachments::delete_files(state.storage.as_ref(), &[id]).await;
    Ok(StatusCode::NO_CONTENT)
}

fn hv(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"))
}

/// Einzelner Byte-Bereich (`bytes=a-b`, `bytes=a-`, `bytes=-n`) als halboffenes Intervall.
/// Mehrfach-Bereiche werden nicht unterstützt (→ `None` = 416).
fn parse_range(value: &str, size: u64) -> Option<std::ops::Range<u64>> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.trim(), end.trim()) {
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            if n == 0 {
                return None;
            }
            (size.saturating_sub(n), size - 1)
        }
        (start, "") => (start.parse().ok()?, size - 1),
        (start, end) => (start.parse().ok()?, end.parse::<u64>().ok()?.min(size - 1)),
    };
    if start > end || start >= size {
        return None;
    }
    Some(start..end + 1)
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(0..10));
        assert_eq!(parse_range("bytes=90-", 100), Some(90..100));
        assert_eq!(parse_range("bytes=-10", 100), Some(90..100));
        assert_eq!(parse_range("bytes=50-500", 100), Some(50..100));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-1", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }
}
