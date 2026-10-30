//! Eingehende Webhooks: Verwaltung pro Textkanal (`MANAGE_WEBHOOKS`, Audit-Log) und Ausführen ohne Anmeldung
//! über `POST /webhooks/{id}/{token}` – Discord-kompatibel genug für die üblichen Integrationen (Gitea,
//! Grafana, Uptime Kuma, Skripte).
//!
//! Datenschutz: `avatar_url` aus der Anfrage wird ignoriert (der Client würde das Bild sonst von einem
//! fremden Server laden und die IP der Lesenden preisgeben); angezeigt wird nur der in Lykos hochgeladene
//! Webhook-Avatar. Aus demselben Grund verwerfen die Embeds alle Bilder (siehe `embeds`).
//! Dateien (`multipart/form-data` mit Dateifeldern) werden nicht angenommen: Anhänge zählen gegen das
//! Kontingent eines Nutzers, und einen solchen gibt es beim Webhook nicht. `payload_json` geht trotzdem.

use axum::{
    Json,
    body::Bytes,
    extract::{FromRequest, Multipart, Path, Query, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::{self, AuthUser},
    automod, embeds, emojis,
    error::{ApiError, ApiResult},
    events,
    models::{self, Message, UserPublic, double_option},
    permissions::{self, Permissions},
    routes::{
        attachments::multipart_error,
        customization,
        messages::{self, NewMessage, Sender},
        users,
    },
    state::AppState,
};

/// Präfix von Webhook-Tokens – für Secret-Scanner erkennbar.
pub const WEBHOOK_TOKEN_PREFIX: &str = "lykw_";
/// Höchstlänge des Namens bzw. des `username` einer Ausführung (Zeichen).
pub const NAME_MAX_CHARS: usize = 80;
/// Höchstzahl Webhooks je Kanal (wie Discord).
pub const MAX_PER_CHANNEL: i64 = 15;

/// Webhook, wie ihn die Verwaltung sieht (ohne Token).
#[derive(Debug, Serialize)]
pub struct Webhook {
    pub id: Uuid,
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_by: Option<UserPublic>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Antwort beim Anlegen und beim Neu-Erzeugen des Tokens: das Token existiert nur hier im Klartext.
#[derive(Serialize)]
pub struct WebhookWithToken {
    #[serde(flatten)]
    webhook: Webhook,
    token: String,
    /// Vollständige Ausführungs-URL zum Kopieren.
    url: String,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    community_id: Uuid,
    channel_id: Uuid,
    name: String,
    avatar_id: Option<Uuid>,
    created_at: OffsetDateTime,
    created_by: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    creator_avatar_id: Option<Uuid>,
}

impl From<Row> for Webhook {
    fn from(r: Row) -> Self {
        Webhook {
            avatar_url: models::webhook_avatar_url(r.id, r.avatar_id),
            created_by: match (r.created_by, r.username) {
                (Some(id), Some(username)) => {
                    Some(UserPublic::new(id, username, r.display_name, r.creator_avatar_id))
                }
                _ => None,
            },
            id: r.id,
            community_id: r.community_id,
            channel_id: r.channel_id,
            name: r.name,
            created_at: r.created_at,
        }
    }
}

macro_rules! webhook_select {
    () => {
        "SELECT w.id, w.community_id, w.channel_id, w.name, w.avatar_id, w.created_at, w.created_by,
            u.username, u.display_name, u.avatar_id AS creator_avatar_id
         FROM webhooks w LEFT JOIN users u ON u.id = w.created_by"
    };
}

async fn by_id(state: &AppState, id: Uuid) -> ApiResult<Option<Webhook>> {
    let row: Option<Row> = sqlx::query_as(concat!(webhook_select!(), " WHERE w.id = $1"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row.map(Webhook::from))
}

fn new_token() -> String {
    format!("{WEBHOOK_TOKEN_PREFIX}{}", auth::random_token(32))
}

fn execute_url(state: &AppState, id: Uuid, token: &str) -> String {
    format!("{}/api/webhooks/{id}/{token}", state.config.public_url.trim_end_matches('/'))
}

/// Name bereinigen: ohne Steuerzeichen, getrimmt, 1–80 Zeichen.
fn normalize_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty() && trimmed.chars().count() <= NAME_MAX_CHARS).then(|| trimmed.to_owned())
}

/// Darf der Nutzer Webhooks im Kanal verwalten? Nur Community-Textkanäle; unsichtbare Kanäle sind `404`.
async fn require_manage(state: &AppState, user_id: Uuid, channel_id: Uuid) -> ApiResult<Uuid> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user_id, channel_id).await?;
    let community_id = ctx.community()?;
    permissions::require(perms, Permissions::MANAGE_WEBHOOKS)?;
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;
    if channel.kind != "text" {
        return Err(ApiError::BadRequest("not_a_text_channel"));
    }
    Ok(community_id)
}

/// Lädt einen Webhook, den der Nutzer verwalten darf.
async fn manageable(state: &AppState, user_id: Uuid, id: Uuid) -> ApiResult<Webhook> {
    let webhook = by_id(state, id).await?.ok_or_else(ApiError::not_found)?;
    require_manage(state, user_id, webhook.channel_id).await?;
    Ok(webhook)
}

/// Verbindungen mit offenen Einstellungen laden die Liste neu.
fn announce(state: &AppState, community_id: Uuid, channel_id: Uuid) {
    state.events.publish(
        events::channel(community_id, channel_id),
        "WEBHOOKS_UPDATE",
        json!({ "community_id": community_id, "channel_id": channel_id }),
    );
}

// --- Verwaltung -----------------------------------------------------------------

/// `GET /channels/{id}/webhooks`
pub async fn list_channel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Webhook>>> {
    require_manage(&state, user.user_id, channel_id).await?;
    let rows: Vec<Row> = sqlx::query_as(concat!(webhook_select!(), " WHERE w.channel_id = $1 ORDER BY w.id"))
        .bind(channel_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.into_iter().map(Webhook::from).collect()))
}

/// `GET /communities/{id}/webhooks` – alle Webhooks der Kanäle, in denen der Nutzer sie verwalten darf.
pub async fn list_community(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Webhook>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    let overrides = permissions::community_overrides(&state.db, community_id).await?;
    let rows: Vec<Row> =
        sqlx::query_as(concat!(webhook_select!(), " WHERE w.community_id = $1 ORDER BY w.id"))
            .bind(community_id)
            .fetch_all(&state.db)
            .await?;
    let need = Permissions::VIEW_CHANNEL | Permissions::MANAGE_WEBHOOKS;
    let allowed = |channel_id: &Uuid| {
        ctx.in_channel(overrides.get(channel_id).map_or(&[], Vec::as_slice)).contains(need)
    };
    Ok(Json(rows.into_iter().filter(|r| allowed(&r.channel_id)).map(Webhook::from).collect()))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
}

/// `POST /channels/{id}/webhooks` – `201` mit `token` und `url` (nur hier im Klartext).
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<WebhookWithToken>)> {
    let community_id = require_manage(&state, user.user_id, channel_id).await?;
    let name = normalize_name(&body.name).ok_or(ApiError::BadRequest("invalid_name"))?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM webhooks WHERE channel_id = $1")
        .bind(channel_id)
        .fetch_one(&state.db)
        .await?;
    if count >= MAX_PER_CHANNEL {
        return Err(ApiError::Conflict("too_many_webhooks"));
    }
    let id = Uuid::now_v7();
    let token = new_token();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO webhooks (id, community_id, channel_id, name, token_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(community_id)
    .bind(channel_id)
    .bind(&name)
    .bind(auth::token_hash(&token))
    .bind(user.user_id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "webhook.create",
        Some(id),
        json!({ "name": name, "channel_id": channel_id }),
    )
    .await?;
    tx.commit().await?;
    announce(&state, community_id, channel_id);

    let webhook = by_id(&state, id).await?.ok_or_else(ApiError::not_found)?;
    let url = execute_url(&state, id, &token);
    Ok((StatusCode::CREATED, Json(WebhookWithToken { webhook, token, url })))
}

/// `GET /webhooks/{id}`
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Webhook>> {
    Ok(Json(manageable(&state, user.user_id, id).await?))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    /// Anderer Textkanal derselben Community (dort ebenfalls `MANAGE_WEBHOOKS`).
    channel_id: Option<Uuid>,
}

/// `PATCH /webhooks/{id}` – Name und/oder Kanal ändern.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Webhook>> {
    let webhook = manageable(&state, user.user_id, id).await?;
    let name = match &body.name {
        Some(name) => Some(normalize_name(name).ok_or(ApiError::BadRequest("invalid_name"))?),
        None => None,
    };
    let channel_id = match body.channel_id {
        Some(channel_id) if channel_id != webhook.channel_id => {
            // Nur innerhalb der Community; fremde Kanäle sind 404 (`require_manage`).
            if require_manage(&state, user.user_id, channel_id).await? != webhook.community_id {
                return Err(ApiError::BadRequest("invalid_channel"));
            }
            Some(channel_id)
        }
        _ => None,
    };
    if name.is_none() && channel_id.is_none() {
        return Ok(Json(webhook));
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE webhooks SET name = COALESCE($2, name), channel_id = COALESCE($3, channel_id) WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;
    let mut changes = serde_json::Map::new();
    if let Some(name) = &name {
        changes.insert("name".into(), json!(name));
    }
    if let Some(channel_id) = channel_id {
        changes.insert("channel_id".into(), json!(channel_id));
        changes.insert("from_channel_id".into(), json!(webhook.channel_id));
    }
    audit::record(
        &mut *tx,
        webhook.community_id,
        user.user_id,
        "webhook.update",
        Some(id),
        Value::Object(changes),
    )
    .await?;
    tx.commit().await?;

    announce(&state, webhook.community_id, webhook.channel_id);
    if let Some(channel_id) = channel_id {
        announce(&state, webhook.community_id, channel_id);
    }
    Ok(Json(by_id(&state, id).await?.ok_or_else(ApiError::not_found)?))
}

/// `POST /webhooks/{id}/token` – neues Token; das alte gilt sofort nicht mehr.
pub async fn regenerate_token(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WebhookWithToken>> {
    let webhook = manageable(&state, user.user_id, id).await?;
    let token = new_token();
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE webhooks SET token_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(auth::token_hash(&token))
        .execute(&mut *tx)
        .await?;
    audit::record(&mut *tx, webhook.community_id, user.user_id, "webhook.token_reset", Some(id), json!({}))
        .await?;
    tx.commit().await?;
    let url = execute_url(&state, id, &token);
    Ok(Json(WebhookWithToken { webhook, token, url }))
}

/// `DELETE /webhooks/{id}` – bisherige Nachrichten bleiben mit dem Namen erhalten.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let webhook = manageable(&state, user.user_id, id).await?;
    let mut tx = state.db.begin().await?;
    let avatar: Option<Option<Uuid>> =
        sqlx::query_scalar("DELETE FROM webhooks WHERE id = $1 RETURNING avatar_id")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    audit::record(
        &mut *tx,
        webhook.community_id,
        user.user_id,
        "webhook.delete",
        Some(id),
        json!({ "name": webhook.name, "channel_id": webhook.channel_id }),
    )
    .await?;
    tx.commit().await?;
    if let Some(Some(avatar)) = avatar {
        attachments::delete_files(state.storage.as_ref(), &[avatar]).await;
    }
    announce(&state, webhook.community_id, webhook.channel_id);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /webhooks/{id}/avatar` – multipart mit Feld `file` (wie Profilbilder).
pub async fn upload_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<Json<Webhook>> {
    let webhook = manageable(&state, user.user_id, id).await?;
    let image = customization::read_image(multipart, users::AVATAR_MAX_BYTES, users::AVATAR_MAX_SIDE).await?;
    let avatar_id = customization::store(&state, image.data).await?;
    swap_avatar(&state, user.user_id, &webhook, Some((avatar_id, image.content_type))).await
}

/// `DELETE /webhooks/{id}/avatar`
pub async fn delete_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Webhook>> {
    let webhook = manageable(&state, user.user_id, id).await?;
    swap_avatar(&state, user.user_id, &webhook, None).await
}

async fn swap_avatar(
    state: &AppState,
    actor: Uuid,
    webhook: &Webhook,
    new: Option<(Uuid, &'static str)>,
) -> ApiResult<Json<Webhook>> {
    let mut tx = state.db.begin().await?;
    // Alte ID im selben Statement auslesen (der Self-Join sieht den Stand vor dem Update).
    let old: Option<Option<Uuid>> = sqlx::query_scalar(
        "UPDATE webhooks w SET avatar_id = $2, avatar_content_type = $3
         FROM webhooks old WHERE w.id = $1 AND old.id = w.id
         RETURNING old.avatar_id",
    )
    .bind(webhook.id)
    .bind(new.map(|(id, _)| id))
    .bind(new.map(|(_, t)| t))
    .fetch_optional(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        webhook.community_id,
        actor,
        "webhook.update",
        Some(webhook.id),
        json!({ "avatar": new.is_some() }),
    )
    .await?;
    tx.commit().await?;
    if let Some(Some(old)) = old {
        attachments::delete_files(state.storage.as_ref(), &[old]).await;
    }
    announce(state, webhook.community_id, webhook.channel_id);
    // Bestehende Nachrichten zeigen den neuen Avatar beim nächsten Laden.
    Ok(Json(by_id(state, webhook.id).await?.ok_or_else(ApiError::not_found)?))
}

/// `GET /webhooks/{id}/avatar/{avatar_id}` – für Mitglieder der Community, nur die aktuelle Avatar-ID.
pub async fn avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, avatar_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT community_id, avatar_content_type FROM webhooks WHERE id = $1 AND avatar_id = $2",
    )
    .bind(id)
    .bind(avatar_id)
    .fetch_optional(&state.db)
    .await?;
    let (community_id, content_type) = row.ok_or_else(ApiError::not_found)?;
    permissions::member_context(&state.db, user.user_id, community_id).await?;
    customization::serve(&state, avatar_id, content_type, "private, max-age=31536000, immutable").await
}

// --- Ausführen (ohne Anmeldung) ---------------------------------------------------

/// Webhook für eine Ausführung: `404 unknown_webhook` oder `401 invalid_webhook_token`; zählt das
/// Rate-Limit des Webhooks.
struct Target {
    id: Uuid,
    community_id: Uuid,
    channel_id: Uuid,
    name: String,
}

async fn authorize(state: &AppState, id: Uuid, token: &str) -> ApiResult<Target> {
    let row: Option<(Uuid, Uuid, String, Vec<u8>)> =
        sqlx::query_as("SELECT community_id, channel_id, name, token_hash FROM webhooks WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (community_id, channel_id, name, hash) = row.ok_or(ApiError::NotFound("unknown_webhook"))?;
    if hash != auth::token_hash(token) {
        return Err(ApiError::Unauthorized("invalid_webhook_token"));
    }
    state.rate_limit.webhook_execute(id)?;
    Ok(Target { id, community_id, channel_id, name })
}

#[derive(Deserialize)]
pub struct ExecuteQuery {
    /// `true`: Antwort `200` mit der Nachricht statt `204`.
    wait: Option<String>,
}

impl ExecuteQuery {
    fn wait(&self) -> bool {
        self.wait.as_deref().is_some_and(|w| w.eq_ignore_ascii_case("true") || w == "1")
    }
}

/// Body einer Ausführung als JSON – direkt oder aus `payload_json` eines Multipart-Formulars.
async fn payload(state: &AppState, req: Request) -> ApiResult<Value> {
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.starts_with("multipart/form-data") {
        let mut multipart =
            Multipart::from_request(req, state).await.map_err(|_| ApiError::BadRequest("invalid_request"))?;
        let mut payload = None;
        while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
            if field.name() == Some("payload_json") {
                let text = field.text().await.map_err(multipart_error)?;
                payload =
                    Some(serde_json::from_str(&text).map_err(|_| ApiError::BadRequest("invalid_json"))?);
            } else if field.file_name().is_some() || field.name().is_some_and(|n| n.starts_with("file")) {
                return Err(ApiError::BadRequest("attachments_not_supported"));
            } else if let Some(name) = field.name().map(str::to_owned) {
                // Einfache Formularfelder (`content`, `username`) wie bei Discord.
                let text = field.text().await.map_err(multipart_error)?;
                let obj = payload.get_or_insert_with(|| json!({}));
                if let Some(map) = obj.as_object_mut() {
                    map.entry(name).or_insert(Value::String(text));
                }
            }
        }
        return payload.ok_or(ApiError::BadRequest("invalid_request"));
    }
    let body = Bytes::from_request(req, state).await.map_err(|_| ApiError::BadRequest("invalid_request"))?;
    serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest("invalid_json"))
}

fn content_of(v: &Value) -> ApiResult<String> {
    let content = match v.get("content") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(ApiError::BadRequest("invalid_content")),
    };
    if content.chars().count() > messages::MAX_CONTENT_CHARS {
        return Err(ApiError::BadRequest("invalid_content"));
    }
    Ok(if content.trim().is_empty() { String::new() } else { content })
}

/// `POST /webhooks/{id}/{token}[?wait=true]` – Nachricht senden. JSON (oder `multipart/form-data` mit
/// `payload_json`): `content`, `username`, `embeds`, `allowed_mentions`; `avatar_url`, `tts`, `flags`,
/// `thread_id` werden ignoriert.
pub async fn execute(
    State(state): State<AppState>,
    Path((id, token)): Path<(Uuid, String)>,
    Query(query): Query<ExecuteQuery>,
    req: Request,
) -> ApiResult<Response> {
    let target = authorize(&state, id, &token).await?;
    let body = payload(&state, req).await?;
    let content = content_of(&body)?;
    let embeds = embeds::parse(body.get("embeds"))?;
    if content.is_empty() && embeds.is_empty() {
        return Err(ApiError::BadRequest("empty_message"));
    }
    let name = body
        .get("username")
        .and_then(Value::as_str)
        .and_then(normalize_name)
        .unwrap_or_else(|| target.name.clone());
    let content = emojis::sanitize_in_community(&state.db, target.community_id, &content).await?;
    let message_id = messages::insert_message(
        &state,
        NewMessage {
            channel_id: target.channel_id,
            community_id: Some(target.community_id),
            sender: Sender::Webhook { id: target.id, name },
            content,
            attachment_ids: Vec::new(),
            reply_to_id: None,
            embeds,
            mentions: embeds::allowed_mentions(body.get("allowed_mentions")),
            automod: Some(automod::Author::webhook(target.id)),
        },
    )
    .await?;
    if !query.wait() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let message =
        models::message_by_id(&state.db, message_id, None).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(message).into_response())
}

/// `GET /webhooks/{id}/{token}` – Webhook-Daten (manche Integrationen prüfen damit die URL).
pub async fn info(
    State(state): State<AppState>,
    Path((id, token)): Path<(Uuid, String)>,
) -> ApiResult<Json<Webhook>> {
    authorize(&state, id, &token).await?;
    let mut webhook = by_id(&state, id).await?.ok_or(ApiError::NotFound("unknown_webhook"))?;
    // Wer nur die URL kennt, erfährt nicht, wer den Webhook angelegt hat.
    webhook.created_by = None;
    Ok(Json(webhook))
}

/// Eine Nachricht dieses Webhooks (sonst `404 unknown_message`).
async fn own_message(state: &AppState, target: &Target, message_id: Uuid) -> ApiResult<Message> {
    models::message_by_id(&state.db, message_id, None)
        .await?
        .filter(|m| m.webhook_id == Some(target.id) && m.channel_id == target.channel_id)
        .ok_or(ApiError::NotFound("unknown_message"))
}

#[derive(Deserialize)]
pub struct EditBody {
    #[serde(default, deserialize_with = "double_option")]
    content: Option<Option<String>>,
    #[serde(default)]
    embeds: Option<Value>,
    #[serde(default)]
    allowed_mentions: Option<Value>,
}

/// `PATCH /webhooks/{id}/{token}/messages/{message_id}` – Text und/oder Embeds ersetzen.
pub async fn edit_message(
    State(state): State<AppState>,
    Path((id, token, message_id)): Path<(Uuid, String, Uuid)>,
    Json(body): Json<EditBody>,
) -> ApiResult<Json<Message>> {
    let target = authorize(&state, id, &token).await?;
    let message = own_message(&state, &target, message_id).await?;
    let content = match body.content {
        None => message.content.clone(),
        Some(content) => content_of(&json!({ "content": content }))?,
    };
    let embeds = match &body.embeds {
        None => message.embeds.clone(),
        Some(v) => embeds::parse(Some(v))?,
    };
    if content.is_empty() && embeds.is_empty() && message.attachments.is_empty() {
        return Err(ApiError::BadRequest("empty_message"));
    }
    let content = emojis::sanitize_in_community(&state.db, target.community_id, &content).await?;
    // AutoMod wie beim Senden (Bearbeitungen ohne Flut-Prüfung); blockiert → `403 automod_blocked`.
    automod::check(
        &state,
        automod::Submission {
            community_id: target.community_id,
            channel_id: target.channel_id,
            author: automod::Author::webhook(target.id),
            webhook_name: message.webhook.as_ref().map(|w| w.name.as_str()),
            content: &content,
            edit: true,
        },
    )
    .await?;
    let policy = embeds::allowed_mentions(body.allowed_mentions.as_ref());

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE messages SET content = $2, embeds = $3, edited_at = now() WHERE id = $1")
        .bind(message_id)
        .bind(&content)
        .bind(sqlx::types::Json(&embeds))
        .execute(&mut *tx)
        .await?;
    messages::store_mentions(
        &mut tx,
        message_id,
        target.channel_id,
        Some(target.community_id),
        &content,
        &policy,
    )
    .await?;
    tx.commit().await?;

    messages::broadcast(&state, "MESSAGE_UPDATE", message_id).await?;
    let message =
        models::message_by_id(&state.db, message_id, None).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(message))
}

/// `DELETE /webhooks/{id}/{token}/messages/{message_id}`
pub async fn delete_message(
    State(state): State<AppState>,
    Path((id, token, message_id)): Path<(Uuid, String, Uuid)>,
) -> ApiResult<StatusCode> {
    let target = authorize(&state, id, &token).await?;
    let message = own_message(&state, &target, message_id).await?;
    messages::delete_message(&state, &message, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_tokens() {
        assert_eq!(normalize_name("  Grafana\u{7} ").as_deref(), Some("Grafana"));
        assert_eq!(normalize_name("   "), None);
        assert_eq!(normalize_name(&"x".repeat(81)), None);
        let token = new_token();
        assert!(token.starts_with(WEBHOOK_TOKEN_PREFIX));
        assert_ne!(token, new_token());
    }

    #[test]
    fn wait_flag() {
        assert!(ExecuteQuery { wait: Some("true".into()) }.wait());
        assert!(ExecuteQuery { wait: Some("True".into()) }.wait());
        assert!(!ExecuteQuery { wait: Some("false".into()) }.wait());
        assert!(!ExecuteQuery { wait: None }.wait());
    }
}
