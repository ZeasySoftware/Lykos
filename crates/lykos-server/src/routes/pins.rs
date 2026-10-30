//! Angepinnte Nachrichten eines Textkanals (Recht `MANAGE_MESSAGES`).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    models::{self, Message},
    permissions::{self, Permissions},
    routes::messages::{broadcast, require_text_channel},
    state::AppState,
};

/// Höchstzahl angepinnter Nachrichten pro Kanal.
pub const MAX_PINS: i64 = 50;

/// `GET /channels/{id}/pins` – angepinnte Nachrichten, zuletzt angepinnte zuerst.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Message>>> {
    permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    require_text_channel(&state, channel_id).await?;
    Ok(Json(models::pinned_messages(&state.db, channel_id, user.user_id).await?))
}

/// `PUT /channels/{id}/pins/{message_id}` – anpinnen (idempotent). Antwort 204.
pub async fn pin(
    State(state): State<AppState>,
    user: AuthUser,
    Path((channel_id, message_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    set_pinned(&state, user.user_id, channel_id, message_id, true).await
}

/// `DELETE /channels/{id}/pins/{message_id}` – lösen (idempotent). Antwort 204.
pub async fn unpin(
    State(state): State<AppState>,
    user: AuthUser,
    Path((channel_id, message_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    set_pinned(&state, user.user_id, channel_id, message_id, false).await
}

async fn set_pinned(
    state: &AppState,
    user_id: Uuid,
    channel_id: Uuid,
    message_id: Uuid,
    pinned: bool,
) -> ApiResult<StatusCode> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user_id, channel_id).await?;
    // In DMs dürfen alle Teilnehmer anpinnen, die dort schreiben dürfen (blockierte DM: nur lesen).
    if ctx.is_dm() {
        permissions::require(perms, Permissions::SEND_MESSAGES)?;
    } else {
        permissions::require(perms, Permissions::MANAGE_MESSAGES)?;
    }
    let row: Option<(Option<Uuid>, bool)> = sqlx::query_as(
        "SELECT author_id, pinned_at IS NOT NULL FROM messages WHERE id = $1 AND channel_id = $2",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((author_id, is_pinned)) = row else {
        return Err(ApiError::not_found());
    };
    if is_pinned == pinned {
        return Ok(StatusCode::NO_CONTENT);
    }

    let mut tx = state.db.begin().await?;
    if pinned {
        // Sperrt die Pins des Kanals nicht, ein seltenes Überschreiten um eins ist unkritisch.
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM messages WHERE channel_id = $1 AND pinned_at IS NOT NULL",
        )
        .bind(channel_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_PINS {
            return Err(ApiError::BadRequest("too_many_pins"));
        }
    }
    let changed = sqlx::query(
        "UPDATE messages SET pinned_at = CASE WHEN $2 THEN now() END
         WHERE id = $1 AND (pinned_at IS NOT NULL) <> $2",
    )
    .bind(message_id)
    .bind(pinned)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Audit-Log gibt es nur für Communities.
    if let Some(community_id) = ctx.community_id {
        audit::record(
            &mut *tx,
            community_id,
            user_id,
            if pinned { "message.pin" } else { "message.unpin" },
            Some(message_id),
            json!({ "channel_id": channel_id, "author_id": author_id }),
        )
        .await?;
    }
    tx.commit().await?;

    broadcast(state, "MESSAGE_UPDATE", message_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
