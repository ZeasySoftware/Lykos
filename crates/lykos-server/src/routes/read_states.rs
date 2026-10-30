//! Lesestand pro Nutzer und Kanal (Ungelesen-Markierung).

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::Scope,
    models, permissions,
    state::AppState,
};

#[derive(Deserialize)]
pub struct ReadBody {
    message_id: Uuid,
}

/// `PUT /channels/{id}/read` – Lesestand setzen, nur vorwärts. Antwort 204.
pub async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<ReadBody>,
) -> ApiResult<StatusCode> {
    let (ctx, _) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;
    if !models::is_text_like(&channel.kind) {
        return Err(ApiError::BadRequest("not_a_text_channel"));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = $1 AND channel_id = $2)")
            .bind(body.message_id)
            .bind(channel_id)
            .fetch_one(&state.db)
            .await?;
    if !exists {
        return Err(ApiError::not_found());
    }

    // UUIDv7 sind zeitlich sortierbar: ein älterer Stand überschreibt nie einen neueren.
    let changed = sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_message_id = EXCLUDED.last_read_message_id
         WHERE read_states.last_read_message_id < EXCLUDED.last_read_message_id",
    )
    .bind(user.user_id)
    .bind(channel_id)
    .bind(body.message_id)
    .execute(&state.db)
    .await?
    .rows_affected();

    if changed > 0 {
        // Andere eigene Geräte synchronisieren.
        state.events.publish(
            Scope::User(user.user_id),
            "READ_STATE_UPDATE",
            json!({
                "channel_id": channel_id,
                "community_id": ctx.community_id,
                "last_read_message_id": body.message_id,
            }),
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Lesestände und neueste Nachricht je Textkanal einer Community – in einer Abfrage.
/// Liefert `(read_states, last_message_ids)`; Kanäle ohne Eintrag fehlen in der jeweiligen Map.
/// Die Filterung auf sichtbare Kanäle übernimmt der Aufrufer.
pub async fn unread_info(
    state: &AppState,
    user_id: Uuid,
    community_id: Uuid,
) -> ApiResult<(HashMap<Uuid, Uuid>, HashMap<Uuid, Uuid>)> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT c.id, rs.last_read_message_id,
                (SELECT m.id FROM messages m WHERE m.channel_id = c.id ORDER BY m.id DESC LIMIT 1)
         FROM channels c
         LEFT JOIN read_states rs ON rs.channel_id = c.id AND rs.user_id = $2
         WHERE c.community_id = $1 AND c.kind = 'text'",
    )
    .bind(community_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let mut read_states = HashMap::new();
    let mut last_message_ids = HashMap::new();
    for (channel_id, read, last) in rows {
        if let Some(read) = read {
            read_states.insert(channel_id, read);
        }
        if let Some(last) = last {
            last_message_ids.insert(channel_id, last);
        }
    }
    Ok((read_states, last_message_ids))
}

/// Ungelesene Nachrichten je Textkanal, die den Nutzer erwähnen (direkt oder per `@everyone`),
/// ohne eigene Nachrichten. Die Filterung auf sichtbare Kanäle übernimmt der Aufrufer.
pub async fn mention_counts(
    state: &AppState,
    user_id: Uuid,
    community_id: Uuid,
) -> ApiResult<HashMap<Uuid, i64>> {
    // Pro Kanal zwei getrennte Zählungen statt eines Scans über die ganze Historie:
    // `@everyone` über den Teilindex `messages_mention_everyone (channel_id, id)`, direkte
    // Erwähnungen über `message_mentions (user_id, message_id)`. Ohne Lesestand gilt die Null-UUID
    // als untere Grenze, so bleibt `id > …` eine Indexbedingung. Nachrichten mit `@everyone` und
    // direkter Erwähnung zählen nur einmal (`NOT m.mention_everyone` im zweiten Teil).
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT c.id, e.n + d.n
         FROM channels c
         LEFT JOIN read_states rs ON rs.channel_id = c.id AND rs.user_id = $2
         CROSS JOIN LATERAL (
             SELECT count(*) AS n FROM messages m
             WHERE m.channel_id = c.id AND m.mention_everyone
               AND m.id > COALESCE(rs.last_read_message_id, '00000000-0000-0000-0000-000000000000')
               AND m.author_id IS DISTINCT FROM $2) e
         CROSS JOIN LATERAL (
             SELECT count(*) AS n FROM message_mentions mm JOIN messages m ON m.id = mm.message_id
             WHERE mm.user_id = $2
               AND mm.message_id > COALESCE(rs.last_read_message_id, '00000000-0000-0000-0000-000000000000')
               AND m.channel_id = c.id AND NOT m.mention_everyone
               AND m.author_id IS DISTINCT FROM $2) d
         WHERE c.community_id = $1 AND c.kind = 'text' AND e.n + d.n > 0",
    )
    .bind(community_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().collect())
}
