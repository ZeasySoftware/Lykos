use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::AuthUser,
    emojis,
    error::{ApiError, ApiResult},
    events, mentions,
    models::{self, Message},
    permissions::{self, Permissions},
    routes::communities::PageQuery,
    state::AppState,
};

pub const MAX_CONTENT_CHARS: usize = 4000;

fn validate_content(content: &str) -> ApiResult<()> {
    let len = content.chars().count();
    if content.trim().is_empty() || len > MAX_CONTENT_CHARS {
        return Err(ApiError::BadRequest("invalid_content"));
    }
    Ok(())
}

pub(crate) async fn require_text_channel(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;
    if models::is_text_like(&channel.kind) { Ok(()) } else { Err(ApiError::BadRequest("not_a_text_channel")) }
}

#[derive(Deserialize)]
pub struct ListQuery {
    before: Option<Uuid>,
    /// Nachrichten um diese ID herum (z. B. Sprung aus der Suche).
    around: Option<Uuid>,
    /// Nur Nachrichten neuer als diese ID (Nachladen nach einem Sprung).
    after: Option<Uuid>,
    limit: Option<i64>,
}

/// Darf die Nachricht `@everyone` auslösen? Nur mit `MENTION_EVERYONE` und nie in DMs
/// (`community_id = None`; dort haben Teilnehmer das Recht ohnehin nicht).
fn mentions_everyone(parsed: &mentions::Parsed, community_id: Option<Uuid>, perms: Permissions) -> bool {
    community_id.is_some() && parsed.everyone && perms.contains(Permissions::MENTION_EVERYONE)
}

/// Erwähnte Nutzer einer Nachricht eintragen: `@name` nur für Mitglieder der Community bzw. – in
/// DMs (`community_id = None`) – nur für Teilnehmer.
async fn insert_mentions(
    conn: &mut PgConnection,
    message_id: Uuid,
    channel_id: Uuid,
    community_id: Option<Uuid>,
    parsed: &mentions::Parsed,
) -> ApiResult<()> {
    if parsed.usernames.is_empty() {
        return Ok(());
    }
    let insert = match community_id {
        Some(community_id) => sqlx::query(
            "INSERT INTO message_mentions (message_id, user_id)
             SELECT $1, m.user_id FROM members m JOIN users u ON u.id = m.user_id
             WHERE m.community_id = $2 AND lower(u.username) = ANY($3)
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id)
        .bind(community_id),
        None => sqlx::query(
            "INSERT INTO message_mentions (message_id, user_id)
             SELECT $1, r.user_id FROM dm_recipients r JOIN users u ON u.id = r.user_id
             WHERE r.channel_id = $2 AND lower(u.username) = ANY($3)
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id)
        .bind(channel_id),
    };
    insert.bind(&parsed.usernames).execute(&mut *conn).await?;
    Ok(())
}

/// Erwähnungen einer bearbeiteten Nachricht neu speichern (Nutzer und `mention_everyone`).
/// Neue Nachrichten setzen `mention_everyone` direkt im INSERT (siehe `create`).
async fn store_mentions(
    conn: &mut PgConnection,
    message_id: Uuid,
    channel_id: Uuid,
    community_id: Option<Uuid>,
    content: &str,
    perms: Permissions,
) -> ApiResult<()> {
    let parsed = mentions::parse(content);
    sqlx::query("DELETE FROM message_mentions WHERE message_id = $1")
        .bind(message_id)
        .execute(&mut *conn)
        .await?;
    insert_mentions(conn, message_id, channel_id, community_id, &parsed).await?;
    sqlx::query("UPDATE messages SET mention_everyone = $2 WHERE id = $1")
        .bind(message_id)
        .bind(mentions_everyone(&parsed, community_id, perms))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Nachricht laden und an alle verteilen, die den Kanal sehen (ohne persönliches `reactions[].me`).
pub(crate) async fn broadcast(state: &AppState, kind: &'static str, message_id: Uuid) -> ApiResult<()> {
    if let Some(message) = models::message_by_id(&state.db, message_id, None).await? {
        state.events.publish(events::channel(message.community_id, message.channel_id), kind, &message);
    }
    Ok(())
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    require_text_channel(&state, channel_id).await?;
    let limit = PageQuery { before: None, limit: q.limit }.limit();
    let viewer = user.user_id;
    let page = match (q.before, q.around, q.after) {
        (before, None, None) => models::messages_page(&state.db, channel_id, before, limit, viewer).await?,
        (None, Some(around), None) => {
            models::messages_around(&state.db, channel_id, around, limit, viewer).await?
        }
        (None, None, Some(after)) => {
            models::messages_after(&state.db, channel_id, after, limit, viewer).await?
        }
        // `before`, `around` und `after` schließen sich gegenseitig aus.
        _ => return Err(ApiError::BadRequest("invalid_request")),
    };
    Ok(Json(page))
}

#[derive(Deserialize)]
pub struct ContentBody {
    content: String,
}

#[derive(Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    content: String,
    /// Vorher hochgeladene, noch unverknüpfte Anhänge (max. 10).
    #[serde(default)]
    attachment_ids: Vec<Uuid>,
    /// Antwort auf eine Nachricht im selben Kanal.
    #[serde(default)]
    reply_to_id: Option<Uuid>,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::SEND_MESSAGES)?;
    require_text_channel(&state, channel_id).await?;
    state.rate_limit.message_send(user.user_id)?;

    let mut attachment_ids = body.attachment_ids;
    attachment_ids.sort_unstable();
    attachment_ids.dedup();
    if attachment_ids.len() > attachments::MAX_PER_MESSAGE {
        return Err(ApiError::BadRequest("too_many_attachments"));
    }
    if attachment_ids.is_empty() {
        validate_content(&body.content)?;
    } else {
        // Mit Anhängen darf der Text leer sein.
        permissions::require(perms, Permissions::ATTACH_FILES)?;
        if body.content.chars().count() > MAX_CONTENT_CHARS {
            return Err(ApiError::BadRequest("invalid_content"));
        }
    }
    if let Some(reply_to_id) = body.reply_to_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = $1 AND channel_id = $2)")
                .bind(reply_to_id)
                .bind(channel_id)
                .fetch_one(&state.db)
                .await?;
        if !exists {
            return Err(ApiError::BadRequest("invalid_reply"));
        }
    }

    let id = Uuid::now_v7();
    let content = if body.content.trim().is_empty() { "" } else { body.content.as_str() };
    // Custom-Emotes nur aus eigenen Communities, sonst `:name:` (siehe `emojis`).
    let content = emojis::sanitize(&state.db, user.user_id, content).await?;
    let content = content.as_str();
    let parsed = mentions::parse(content);
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, mention_everyone)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(user.user_id)
    .bind(content)
    .bind(body.reply_to_id)
    .bind(mentions_everyone(&parsed, ctx.community_id, perms))
    .execute(&mut *tx)
    .await?;
    if !attachment_ids.is_empty() {
        // Nur eigene, unverknüpfte Uploads aus genau diesem Kanal; sonst wird nichts gesendet.
        let linked = sqlx::query(
            "UPDATE attachments SET message_id = $1
             WHERE id = ANY($2) AND uploader_id = $3 AND channel_id = $4 AND message_id IS NULL",
        )
        .bind(id)
        .bind(&attachment_ids)
        .bind(user.user_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if linked != attachment_ids.len() as u64 {
            return Err(ApiError::BadRequest("invalid_attachments"));
        }
    }
    insert_mentions(&mut tx, id, channel_id, ctx.community_id, &parsed).await?;
    tx.commit().await?;

    broadcast(&state, "MESSAGE_CREATE", id).await?;
    let message =
        models::message_by_id(&state.db, id, Some(user.user_id)).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(message)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(message_id): Path<Uuid>,
    Json(body): Json<ContentBody>,
) -> ApiResult<Json<Message>> {
    let message =
        models::message_by_id(&state.db, message_id, None).await?.ok_or_else(ApiError::not_found)?;
    // Sichtbarkeit prüfen, bevor wir verraten, wem die Nachricht gehört.
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, message.channel_id).await?;
    if message.author.as_ref().map(|a| a.id) != Some(user.user_id) {
        return Err(ApiError::forbidden());
    }
    // Bearbeiten ist Schreiben: Wer im Kanal nicht (mehr) schreiben darf – entzogenes Recht, blockierte
    // DM –, ändert auch seine alten Nachrichten nicht.
    permissions::require(perms, Permissions::SEND_MESSAGES)?;
    // Systemnachrichten (z. B. „X ist beigetreten“) haben keinen bearbeitbaren Text.
    if message.kind != "default" {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    validate_content(&body.content)?;
    let content = emojis::sanitize(&state.db, user.user_id, &body.content).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE messages SET content = $2, edited_at = now() WHERE id = $1")
        .bind(message_id)
        .bind(&content)
        .execute(&mut *tx)
        .await?;
    store_mentions(&mut tx, message_id, message.channel_id, ctx.community_id, &content, perms).await?;
    tx.commit().await?;

    broadcast(&state, "MESSAGE_UPDATE", message_id).await?;
    let message = models::message_by_id(&state.db, message_id, Some(user.user_id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(message))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(message_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let message =
        models::message_by_id(&state.db, message_id, None).await?.ok_or_else(ApiError::not_found)?;
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, message.channel_id).await?;
    let is_author = message.author.as_ref().map(|a| a.id) == Some(user.user_id);
    if !is_author {
        permissions::require(perms, Permissions::MANAGE_MESSAGES)?;
    }

    let mut tx = state.db.begin().await?;
    let attachment_ids: Vec<Uuid> =
        sqlx::query_scalar("DELETE FROM attachments WHERE message_id = $1 RETURNING id")
            .bind(message_id)
            .fetch_all(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM messages WHERE id = $1").bind(message_id).execute(&mut *tx).await?;
    if !is_author {
        // Moderation protokollieren, aber ohne Nachrichteninhalt.
        audit::record(
            &mut *tx,
            ctx.community()?,
            user.user_id,
            "message.delete",
            Some(message_id),
            json!({ "channel_id": message.channel_id, "author_id": message.author.map(|a| a.id) }),
        )
        .await?;
    }
    tx.commit().await?;
    attachments::delete_files(state.storage.as_ref(), &attachment_ids).await;

    state.events.publish(
        events::channel(ctx.community_id, message.channel_id),
        "MESSAGE_DELETE",
        json!({ "id": message_id, "channel_id": message.channel_id, "community_id": ctx.community_id }),
    );
    Ok(StatusCode::NO_CONTENT)
}
