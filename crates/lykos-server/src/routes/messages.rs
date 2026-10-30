use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::AuthUser,
    automod,
    embeds::{self, Embed, MentionPolicy, UserMentions},
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

/// Erwähnungs-Regeln für Nachrichten von Nutzern und Bots: alle erwähnten Mitglieder; `@everyone` nur mit
/// `MENTION_EVERYONE` und nie in DMs (`community_id = None`; dort haben Teilnehmer das Recht ohnehin nicht).
fn user_policy(community_id: Option<Uuid>, perms: Permissions) -> MentionPolicy {
    MentionPolicy {
        users: UserMentions::All,
        everyone: community_id.is_some() && perms.contains(Permissions::MENTION_EVERYONE),
    }
}

/// Erwähnte Nutzer einer Nachricht eintragen: `@name` nur für Mitglieder der Community bzw. – in
/// DMs (`community_id = None`) – nur für Teilnehmer; mit `policy` ggf. nur bestimmte oder niemand.
async fn insert_mentions(
    conn: &mut PgConnection,
    message_id: Uuid,
    channel_id: Uuid,
    community_id: Option<Uuid>,
    parsed: &mentions::Parsed,
    policy: &MentionPolicy,
) -> ApiResult<()> {
    if parsed.usernames.is_empty() || policy.users == UserMentions::None {
        return Ok(());
    }
    let insert = match community_id {
        Some(community_id) => sqlx::query(
            "INSERT INTO message_mentions (message_id, user_id)
             SELECT $1, m.user_id FROM members m JOIN users u ON u.id = m.user_id
             WHERE m.community_id = $2 AND lower(u.username) = ANY($3)
               AND ($4::uuid[] IS NULL OR m.user_id = ANY($4))
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id)
        .bind(community_id),
        None => sqlx::query(
            "INSERT INTO message_mentions (message_id, user_id)
             SELECT $1, r.user_id FROM dm_recipients r JOIN users u ON u.id = r.user_id
             WHERE r.channel_id = $2 AND lower(u.username) = ANY($3)
               AND ($4::uuid[] IS NULL OR r.user_id = ANY($4))
             ON CONFLICT DO NOTHING",
        )
        .bind(message_id)
        .bind(channel_id),
    };
    insert.bind(&parsed.usernames).bind(policy.user_ids()).execute(&mut *conn).await?;
    Ok(())
}

/// Erwähnungen einer bearbeiteten Nachricht neu speichern (Nutzer und `mention_everyone`).
/// Neue Nachrichten setzen `mention_everyone` direkt im INSERT (siehe `insert_message`).
pub(crate) async fn store_mentions(
    conn: &mut PgConnection,
    message_id: Uuid,
    channel_id: Uuid,
    community_id: Option<Uuid>,
    content: &str,
    policy: &MentionPolicy,
) -> ApiResult<()> {
    let parsed = mentions::parse(content);
    sqlx::query("DELETE FROM message_mentions WHERE message_id = $1")
        .bind(message_id)
        .execute(&mut *conn)
        .await?;
    insert_mentions(conn, message_id, channel_id, community_id, &parsed, policy).await?;
    sqlx::query("UPDATE messages SET mention_everyone = $2 WHERE id = $1")
        .bind(message_id)
        .bind(parsed.everyone && policy.everyone)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// AutoMod der Community für eine Bearbeitung durch einen Nutzer oder Bot prüfen (nur in Community-Kanälen, nicht
/// in DMs) – vor dem Speichern, mit dem endgültigen Inhalt. Blockiert → `403 automod_blocked`, nichts wird
/// gespeichert. Neue Nachrichten prüft `insert_message`.
async fn automod_check(
    state: &AppState,
    ctx: &permissions::ChannelContext,
    user_id: Uuid,
    channel_id: Uuid,
    content: &str,
    edit: bool,
) -> ApiResult<()> {
    let Some(member) = &ctx.member else { return Ok(()) };
    let submission = automod::Submission {
        community_id: member.community_id,
        channel_id,
        author: automod::Author::member(user_id, member),
        webhook_name: None,
        content,
        edit,
    };
    automod::check(state, submission).await
}

/// Nachricht laden und an alle verteilen, die den Kanal sehen (ohne persönliches `reactions[].me`).
pub(crate) async fn broadcast(state: &AppState, kind: &'static str, message_id: Uuid) -> ApiResult<()> {
    if let Some(message) = models::message_by_id(&state.db, message_id, None).await? {
        state.events.publish(events::channel(message.community_id, message.channel_id), kind, &message);
    }
    Ok(())
}

/// Absender einer neuen Nachricht.
pub(crate) enum Sender {
    /// Mensch oder Bot (beide sind Nutzer).
    User(Uuid),
    /// Eingehender Webhook mit Anzeigenamen (`author_id` bleibt `NULL`).
    Webhook { id: Uuid, name: String },
}

/// Alles für eine neue Nachricht – Rechte, Rate-Limit und Inhalt (Länge, Emotes) hat der Aufrufer geprüft.
pub(crate) struct NewMessage {
    pub channel_id: Uuid,
    pub community_id: Option<Uuid>,
    pub sender: Sender,
    pub content: String,
    /// Eigene, unverknüpfte Uploads (nur bei `Sender::User`).
    pub attachment_ids: Vec<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub embeds: Vec<Embed>,
    pub mentions: MentionPolicy,
    /// Für den AutoMod: Mitglied (Mensch oder Bot, mit Rollen und Ausnahmen) bzw. Webhook. `None` = keine Prüfung
    /// (DMs, in denen es keinen AutoMod gibt).
    pub automod: Option<automod::Author>,
}

/// Gemeinsamer Erstellpfad aller Nachrichten (Nutzer, Bots, Webhooks): speichern, Anhänge verknüpfen,
/// Erwähnungen eintragen und `MESSAGE_CREATE` verteilen. Gibt die ID zurück.
///
/// Prüfpunkte, die für jede Nachricht gelten sollen (z. B. AutoMod), gehören hierher – vor das INSERT.
pub(crate) async fn insert_message(state: &AppState, new: NewMessage) -> ApiResult<Uuid> {
    if let Some(reply_to_id) = new.reply_to_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = $1 AND channel_id = $2)")
                .bind(reply_to_id)
                .bind(new.channel_id)
                .fetch_one(&state.db)
                .await?;
        if !exists {
            return Err(ApiError::BadRequest("invalid_reply"));
        }
    }
    let (author_id, webhook_id, webhook_name) = match &new.sender {
        Sender::User(id) => (Some(*id), None, None),
        Sender::Webhook { id, name } => (None, Some(*id), Some(name.as_str())),
    };
    // AutoMod mit dem endgültigen Inhalt, bevor etwas gespeichert wird (blockiert → `403 automod_blocked`).
    if let (Some(community_id), Some(author)) = (new.community_id, new.automod.clone()) {
        let submission = automod::Submission {
            community_id,
            channel_id: new.channel_id,
            author,
            webhook_name,
            content: &new.content,
            edit: false,
        };
        automod::check(state, submission).await?;
    }

    let id = Uuid::now_v7();
    let parsed = mentions::parse(&new.content);
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, mention_everyone,
                               webhook_id, webhook_name, embeds)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(new.channel_id)
    .bind(author_id)
    .bind(&new.content)
    .bind(new.reply_to_id)
    .bind(parsed.everyone && new.mentions.everyone)
    .bind(webhook_id)
    .bind(webhook_name)
    .bind(sqlx::types::Json(&new.embeds))
    .execute(&mut *tx)
    .await?;
    if !new.attachment_ids.is_empty() {
        let Some(uploader) = author_id else {
            return Err(ApiError::BadRequest("invalid_attachments"));
        };
        // Nur eigene, unverknüpfte Uploads aus genau diesem Kanal; sonst wird nichts gesendet.
        let linked = sqlx::query(
            "UPDATE attachments SET message_id = $1
             WHERE id = ANY($2) AND uploader_id = $3 AND channel_id = $4 AND message_id IS NULL",
        )
        .bind(id)
        .bind(&new.attachment_ids)
        .bind(uploader)
        .bind(new.channel_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if linked != new.attachment_ids.len() as u64 {
            return Err(ApiError::BadRequest("invalid_attachments"));
        }
    }
    insert_mentions(&mut tx, id, new.channel_id, new.community_id, &parsed, &new.mentions).await?;
    tx.commit().await?;

    broadcast(state, "MESSAGE_CREATE", id).await?;
    Ok(id)
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
    /// Embeds (Discord-Format, nur für Bots; siehe `embeds`).
    #[serde(default)]
    embeds: Option<Value>,
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

    // Embeds senden nur Bots; die Oberfläche hat dafür keinen Editor.
    let embeds = match (&body.embeds, user.is_bot) {
        (None, _) => Vec::new(),
        (Some(v), true) => embeds::parse(Some(v))?,
        (Some(_), false) => return Err(ApiError::BadRequest("invalid_request")),
    };
    let mut attachment_ids = body.attachment_ids;
    attachment_ids.sort_unstable();
    attachment_ids.dedup();
    if attachment_ids.len() > attachments::MAX_PER_MESSAGE {
        return Err(ApiError::BadRequest("too_many_attachments"));
    }
    if attachment_ids.is_empty() && embeds.is_empty() {
        validate_content(&body.content)?;
    } else {
        // Mit Anhängen oder Embeds darf der Text leer sein.
        if !attachment_ids.is_empty() {
            permissions::require(perms, Permissions::ATTACH_FILES)?;
        }
        if body.content.chars().count() > MAX_CONTENT_CHARS {
            return Err(ApiError::BadRequest("invalid_content"));
        }
    }

    let content = if body.content.trim().is_empty() { "" } else { body.content.as_str() };
    // Custom-Emotes nur aus eigenen Communities, sonst `:name:` (siehe `emojis`).
    let content = emojis::sanitize(&state.db, user.user_id, content).await?;
    let id = insert_message(
        &state,
        NewMessage {
            channel_id,
            community_id: ctx.community_id,
            sender: Sender::User(user.user_id),
            content,
            attachment_ids,
            reply_to_id: body.reply_to_id,
            embeds,
            mentions: user_policy(ctx.community_id, perms),
            automod: ctx.member.as_ref().map(|m| automod::Author::member(user.user_id, m)),
        },
    )
    .await?;
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
    automod_check(&state, &ctx, user.user_id, message.channel_id, &content, true).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE messages SET content = $2, edited_at = now() WHERE id = $1")
        .bind(message_id)
        .bind(&content)
        .execute(&mut *tx)
        .await?;
    let policy = user_policy(ctx.community_id, perms);
    store_mentions(&mut tx, message_id, message.channel_id, ctx.community_id, &content, &policy).await?;
    tx.commit().await?;

    broadcast(&state, "MESSAGE_UPDATE", message_id).await?;
    let message = models::message_by_id(&state.db, message_id, Some(user.user_id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(message))
}

/// Nachricht samt Anhängen löschen und `MESSAGE_DELETE` verteilen. Moderationsfälle protokolliert der
/// Aufrufer (in derselben Transaktion ist hier nicht nötig: das Audit-Log enthält keinen Inhalt).
pub(crate) async fn delete_message(
    state: &AppState,
    message: &Message,
    moderator: Option<Uuid>,
) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    let attachment_ids: Vec<Uuid> =
        sqlx::query_scalar("DELETE FROM attachments WHERE message_id = $1 RETURNING id")
            .bind(message.id)
            .fetch_all(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM messages WHERE id = $1").bind(message.id).execute(&mut *tx).await?;
    if let (Some(moderator), Some(community_id)) = (moderator, message.community_id) {
        // Moderation protokollieren, aber ohne Nachrichteninhalt.
        audit::record(
            &mut *tx,
            community_id,
            moderator,
            "message.delete",
            Some(message.id),
            json!({
                "channel_id": message.channel_id,
                "author_id": message.author.as_ref().map(|a| a.id),
                "webhook_id": message.webhook_id,
            }),
        )
        .await?;
    }
    tx.commit().await?;
    attachments::delete_files(state.storage.as_ref(), &attachment_ids).await;

    state.events.publish(
        events::channel(message.community_id, message.channel_id),
        "MESSAGE_DELETE",
        json!({ "id": message.id, "channel_id": message.channel_id, "community_id": message.community_id }),
    );
    Ok(())
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
        // Fremde Nachrichten – auch von Webhooks – nur mit MANAGE_MESSAGES (in DMs gibt es das Recht nicht).
        permissions::require(perms, Permissions::MANAGE_MESSAGES)?;
        ctx.community()?;
    }
    delete_message(&state, &message, (!is_author).then_some(user.user_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
