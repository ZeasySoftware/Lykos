//! Banns: Nutzer dauerhaft aus einer Community ausschließen (`BAN_MEMBERS`).
//!
//! Bannen wirft wie ein Kick sofort hinaus (Gateway-Abo, Voice) und sperrt zusätzlich den Wiederbeitritt
//! über Einladungen (`403 banned`), bis der Bann aufgehoben wird. Es gilt dieselbe Rollenhierarchie wie
//! beim Kick; den Besitzer und sich selbst kann niemand bannen.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::{self, Control, Scope},
    models::{self, UserPublic},
    permissions::{self, Permissions},
    routes::roles,
    state::AppState,
    voice,
};

/// Höchstlänge des Bann-Grunds.
pub const REASON_MAX_CHARS: usize = 512;
/// Nachrichten höchstens der letzten 7 Tage mitlöschen (wie Discord).
pub const MAX_DELETE_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Nachrichten pro Lösch-Transaktion.
const DELETE_BATCH: i64 = 500;

#[derive(Deserialize)]
pub struct BanBody {
    /// Optionaler Grund (max. 512 Zeichen), sichtbar in der Bannliste und im Audit-Log.
    reason: Option<String>,
    /// Nachrichten des Nutzers in der Community aus diesem Zeitraum (Sekunden, 0–604800) löschen.
    #[serde(default)]
    delete_message_seconds: i64,
}

/// `PUT /communities/{id}/bans/{user_id}` – Nutzer bannen (auch Nicht-Mitglieder, z. B. nach dem
/// Verlassen). Ein erneuter Aufruf aktualisiert Grund, Moderator und Zeitpunkt.
pub async fn ban(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<BanBody>,
) -> ApiResult<StatusCode> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::BAN_MEMBERS)?;
    if target_id == user.user_id {
        return Err(ApiError::BadRequest("cannot_ban_self"));
    }
    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    if target_id == community.owner_id {
        return Err(ApiError::forbidden());
    }
    // Rollenhierarchie wie beim Kick: nur Mitglieder, deren höchste Rolle unter der eigenen steht
    // (Nicht-Mitglieder haben keine Rollen).
    roles::require_outranks_member(&state.db, &ctx, target_id).await?;
    let reason = crate::auth::normalize_text(body.reason, REASON_MAX_CHARS, "invalid_reason")?;
    if !(0..=MAX_DELETE_SECONDS).contains(&body.delete_message_seconds) {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    if models::user_by_id(&state.db, target_id).await?.is_none() {
        return Err(ApiError::not_found());
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO community_bans (community_id, user_id, banned_by, reason) VALUES ($1, $2, $3, $4)
         ON CONFLICT (community_id, user_id)
         DO UPDATE SET banned_by = EXCLUDED.banned_by, reason = EXCLUDED.reason, created_at = now()",
    )
    .bind(community_id)
    .bind(target_id)
    .bind(user.user_id)
    .bind(&reason)
    .execute(&mut *tx)
    .await?;
    let was_member = sqlx::query("DELETE FROM members WHERE community_id = $1 AND user_id = $2")
        .bind(community_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "member.ban",
        Some(target_id),
        json!({ "reason": reason, "delete_message_seconds": body.delete_message_seconds }),
    )
    .await?;
    tx.commit().await?;

    if was_member {
        state.events.publish(
            events::community(community_id),
            "MEMBER_REMOVE",
            json!({ "community_id": community_id, "user_id": target_id }),
        );
        state.events.control(Scope::User(target_id), Control::Left { user_id: target_id, community_id });
        voice::member_removed(&state, community_id, target_id).await?;
    }
    if body.delete_message_seconds > 0 {
        delete_recent_messages(&state, community_id, target_id, body.delete_message_seconds).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Löscht die Nachrichten von `user_id` in der Community aus den letzten `seconds` Sekunden – in Portionen
/// mit je eigener kurzer Transaktion – und meldet jede wie beim normalen Löschen per `MESSAGE_DELETE`.
/// Liefert die Anzahl gelöschter Nachrichten.
pub async fn delete_recent_messages(
    state: &AppState,
    community_id: Uuid,
    user_id: Uuid,
    seconds: i64,
) -> ApiResult<u64> {
    let mut total = 0;
    loop {
        let mut tx = state.db.begin().await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT m.id FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE c.community_id = $1 AND m.author_id = $2
               AND m.created_at > now() - make_interval(secs => $3::double precision)
             ORDER BY m.id LIMIT $4 FOR UPDATE OF m",
        )
        .bind(community_id)
        .bind(user_id)
        .bind(seconds as f64)
        .bind(DELETE_BATCH)
        .fetch_all(&mut *tx)
        .await?;
        if ids.is_empty() {
            return Ok(total);
        }
        // Anhänge verschwinden per Kaskade aus der Datenbank, die Dateien räumen wir selbst ab.
        let files: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM attachments WHERE message_id = ANY($1)")
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?;
        let deleted: Vec<(Uuid, Uuid)> =
            sqlx::query_as("DELETE FROM messages WHERE id = ANY($1) RETURNING id, channel_id")
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await?;
        tx.commit().await?;
        attachments::delete_files(state.storage.as_ref(), &files).await;

        for (id, channel_id) in &deleted {
            state.events.publish(
                events::channel(community_id, *channel_id),
                "MESSAGE_DELETE",
                json!({ "id": id, "channel_id": channel_id, "community_id": community_id }),
            );
        }
        total += deleted.len() as u64;
        if (ids.len() as i64) < DELETE_BATCH {
            return Ok(total);
        }
    }
}

/// `DELETE /communities/{id}/bans/{user_id}` – Bann aufheben; danach kann der Nutzer über eine Einladung
/// wieder beitreten.
pub async fn unban(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::BAN_MEMBERS)?;

    let mut tx = state.db.begin().await?;
    let removed = sqlx::query("DELETE FROM community_bans WHERE community_id = $1 AND user_id = $2")
        .bind(community_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(ApiError::not_found());
    }
    audit::record(&mut *tx, community_id, user.user_id, "member.unban", Some(target_id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Eintrag der Bannliste.
#[derive(Serialize)]
pub struct Ban {
    user: UserPublic,
    reason: Option<String>,
    /// Wer gebannt hat; `null`, wenn das Konto inzwischen gelöscht ist.
    banned_by: Option<UserPublic>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(FromRow)]
struct BanRow {
    user_id: Uuid,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    reason: Option<String>,
    banned_by: Option<Uuid>,
    mod_username: Option<String>,
    mod_display_name: Option<String>,
    mod_avatar_id: Option<Uuid>,
    created_at: OffsetDateTime,
}

/// `GET /communities/{id}/bans` – alle Banns der Community, neueste zuerst.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Ban>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::BAN_MEMBERS)?;

    let rows: Vec<BanRow> = sqlx::query_as(
        "SELECT b.user_id, u.username, u.display_name, u.avatar_id, b.reason, b.banned_by,
                m.username AS mod_username, m.display_name AS mod_display_name, m.avatar_id AS mod_avatar_id,
                b.created_at
         FROM community_bans b
         JOIN users u ON u.id = b.user_id
         LEFT JOIN users m ON m.id = b.banned_by
         WHERE b.community_id = $1
         ORDER BY b.created_at DESC",
    )
    .bind(community_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Ban {
                user: UserPublic::new(r.user_id, r.username, r.display_name, r.avatar_id),
                reason: r.reason,
                banned_by: match (r.banned_by, r.mod_username) {
                    (Some(id), Some(name)) => {
                        Some(UserPublic::new(id, name, r.mod_display_name, r.mod_avatar_id))
                    }
                    _ => None,
                },
                created_at: r.created_at,
            })
            .collect(),
    ))
}

/// Ist der Nutzer in der Community gebannt?
pub async fn is_banned(db: impl sqlx::PgExecutor<'_>, community_id: Uuid, user_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM community_bans WHERE community_id = $1 AND user_id = $2)",
    )
    .bind(community_id)
    .bind(user_id)
    .fetch_one(db)
    .await?)
}
