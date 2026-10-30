//! Bot-Konten: Nutzer mit `users.is_bot = true` und Besitzer (`bot_owner_id`), ohne Passwort, Anmeldung,
//! E-Mail oder 2FA. Sie authentifizieren sich mit `Authorization: Bot <token>` (REST und Gateway) und laufen
//! durch dieselben Rechteprüfungen wie Menschen. Welche Endpunkte Bots grundsätzlich nicht nutzen dürfen
//! (Konto, Sicherheit, Freunde, DMs eröffnen, Instanz-Verwaltung …), regelt zentral `reject_forbidden`.
//!
//! Jeder Nutzer verwaltet seine Bots im Einstellungs-Tab „Entwickler“ (höchstens `MAX_BOTS_PER_USER`).
//! Instanz-Admins können Bots mit der Laufzeit-Einstellung `allow_bots = false` ganz abschalten.
//! Zu einer Community hinzufügen darf, wer dort `MANAGE_COMMUNITY` hat – eigene Bots immer, fremde nur,
//! wenn ihr Besitzer sie als öffentlich markiert hat.

use axum::{
    Json,
    extract::{Multipart, Path, Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    accounts, audit,
    auth::{self, AuthUser},
    error::{ApiError, ApiResult, is_unique_violation},
    events::{Control, Scope},
    instance_settings,
    models::{self, Member, double_option},
    permissions::{self, Permissions},
    routes::{invites, users},
    state::AppState,
};

/// Höchstzahl Bots je Nutzer.
pub const MAX_BOTS_PER_USER: i64 = 10;
/// Höchstlänge der Beschreibung (Zeichen).
pub const DESCRIPTION_MAX_CHARS: usize = 400;

// --- Zentrale Sperre für Bot-Anfragen -------------------------------------------

/// Liegt `path` unter `prefix` (genau oder mit weiterem Segment)?
fn under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Endpunkte, die Bots nie nutzen dürfen – unabhängig von Rechten in Communities:
/// Konto, Sitzungen, Sicherheit und Profil (`/auth/*` außer `GET /auth/me`), Freunde, DMs eröffnen und
/// DM-Einstellungen, Bot-Verwaltung, Instanz-Verwaltung, Feedback, Sprachkanäle/Anrufe, Communities anlegen,
/// Einladungen annehmen und Besitz übertragen.
pub fn bot_forbidden(method: &Method, path: &str) -> bool {
    let path = path.strip_prefix("/api").unwrap_or(path);
    if path == "/auth/me" {
        return method != Method::GET;
    }
    const DENIED: [&str; 8] = [
        "/auth",
        "/relationships",
        "/bots",
        "/admin",
        "/feedback",
        "/voice",
        "/dms/contacts",
        "/dms/settings",
    ];
    if DENIED.iter().any(|p| under(path, p)) {
        return true;
    }
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["dms"] | ["communities"] => method == Method::POST,
        ["invites", _, "accept"] => true,
        ["communities", _, "transfer"] => true,
        ["channels", _, "voice", ..] | ["channels", _, "call", ..] => true,
        _ => false,
    }
}

/// Middleware für alle API-Routen: Anfragen mit `Authorization: Bot …` auf gesperrte Endpunkte enden mit
/// `403 bots_not_allowed`, bevor ein Handler läuft.
pub async fn reject_forbidden(req: Request, next: Next) -> Response {
    if auth::is_bot_request(req.headers()) && bot_forbidden(req.method(), req.uri().path()) {
        return ApiError::Forbidden("bots_not_allowed").into_response();
    }
    next.run(req).await
}

// --- Modell ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Bot {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub description: Option<String>,
    pub is_public: bool,
    pub owner_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Antwort beim Anlegen und Neu-Erzeugen: das Token existiert nur hier im Klartext.
#[derive(Serialize)]
pub struct BotWithToken {
    #[serde(flatten)]
    bot: Bot,
    token: String,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    bot_owner_id: Uuid,
    created_at: OffsetDateTime,
    description: Option<String>,
    is_public: bool,
}

impl From<Row> for Bot {
    fn from(r: Row) -> Self {
        Bot {
            avatar_url: models::avatar_url(r.id, r.avatar_id),
            id: r.id,
            username: r.username,
            display_name: r.display_name,
            description: r.description,
            is_public: r.is_public,
            owner_id: r.bot_owner_id,
            created_at: r.created_at,
        }
    }
}

macro_rules! bot_select {
    () => {
        "SELECT u.id, u.username, u.display_name, u.avatar_id, u.bot_owner_id, u.created_at,
            b.description, b.is_public
         FROM users u JOIN bots b ON b.user_id = u.id"
    };
}

async fn by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<Bot>> {
    let row: Option<Row> =
        sqlx::query_as(concat!(bot_select!(), " WHERE u.id = $1")).bind(id).fetch_optional(db).await?;
    Ok(row.map(Bot::from))
}

/// Eigene Bots des Nutzers (auch für den Datenexport, ohne Token).
pub async fn owned_by(db: impl PgExecutor<'_>, owner_id: Uuid) -> ApiResult<Vec<Bot>> {
    let rows: Vec<Row> =
        sqlx::query_as(concat!(bot_select!(), " WHERE u.bot_owner_id = $1 ORDER BY u.created_at"))
            .bind(owner_id)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(Bot::from).collect())
}

/// Eigener Bot des Nutzers, sonst `404`.
async fn own(state: &AppState, owner_id: Uuid, id: Uuid) -> ApiResult<Bot> {
    by_id(&state.db, id).await?.filter(|b| b.owner_id == owner_id).ok_or_else(ApiError::not_found)
}

fn new_token() -> String {
    format!("{}{}", auth::BOT_TOKEN_PREFIX, auth::random_token(32))
}

async fn require_bots_enabled(state: &AppState) -> ApiResult<()> {
    if instance_settings::load(&state.db, &state.config).await?.allow_bots {
        Ok(())
    } else {
        Err(ApiError::Forbidden("bots_disabled"))
    }
}

// --- Verwaltung (Besitzer) -----------------------------------------------------

/// `GET /bots` – eigene Bots.
pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<Bot>>> {
    Ok(Json(owned_by(&state.db, user.user_id).await?))
}

#[derive(Deserialize)]
pub struct CreateBody {
    username: String,
    display_name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    is_public: bool,
}

/// `POST /bots` – `201` mit `token` (nur hier im Klartext).
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<BotWithToken>)> {
    require_bots_enabled(&state).await?;
    let username = body.username.trim().to_owned();
    auth::validate_username(&username)?;
    let display_name = auth::normalize_display_name(body.display_name)?;
    let description = auth::normalize_text(body.description, DESCRIPTION_MAX_CHARS, "invalid_description")?;

    let mut tx = state.db.begin().await?;
    // Besitzer sperren, damit parallele Anfragen die Obergrenze nicht überschreiten.
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE").bind(user.user_id).execute(&mut *tx).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE bot_owner_id = $1")
        .bind(user.user_id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_BOTS_PER_USER {
        return Err(ApiError::Conflict("too_many_bots"));
    }
    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_bot, bot_owner_id)
         VALUES ($1, $2, $3, NULL, true, $4)",
    )
    .bind(id)
    .bind(&username)
    .bind(&display_name)
    .bind(user.user_id)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("username_taken")),
        Err(e) => return Err(e.into()),
    }
    let token = new_token();
    sqlx::query(
        "INSERT INTO bots (user_id, description, is_public, token_hash, token_id) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(&description)
    .bind(body.is_public)
    .bind(auth::token_hash(&token))
    .bind(Uuid::now_v7())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(bot_id = %id, owner_id = %user.user_id, "Bot angelegt");

    let bot = by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(BotWithToken { bot, token })))
}

/// `GET /bots/{id}` – eigener oder öffentlicher Bot (Vorschau vor dem Hinzufügen).
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Bot>> {
    let bot = by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?;
    if bot.owner_id != user.user_id && !bot.is_public {
        return Err(ApiError::not_found());
    }
    Ok(Json(bot))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(default, deserialize_with = "double_option")]
    display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    description: Option<Option<String>>,
    is_public: Option<bool>,
}

/// `PATCH /bots/{id}` – Anzeigename, Beschreibung, öffentlich.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Bot>> {
    let bot = own(&state, user.user_id, id).await?;
    let mut tx = state.db.begin().await?;
    if let Some(name) = body.display_name {
        sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
            .bind(id)
            .bind(auth::normalize_display_name(name)?)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(description) = body.description {
        sqlx::query("UPDATE bots SET description = $2 WHERE user_id = $1")
            .bind(id)
            .bind(auth::normalize_text(description, DESCRIPTION_MAX_CHARS, "invalid_description")?)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(is_public) = body.is_public {
        sqlx::query("UPDATE bots SET is_public = $2 WHERE user_id = $1")
            .bind(id)
            .bind(is_public)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    users::announce_user_update(&state, bot.id).await?;
    Ok(Json(by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?))
}

/// `POST /bots/{id}/token` – neues Token; das alte gilt sofort nicht mehr, Gateway-Verbindungen werden getrennt.
pub async fn regenerate_token(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BotWithToken>> {
    let bot = own(&state, user.user_id, id).await?;
    let token = new_token();
    let old: Uuid = sqlx::query_scalar(
        "UPDATE bots b SET token_hash = $2, token_id = $3 FROM bots old
         WHERE b.user_id = $1 AND old.user_id = b.user_id RETURNING old.token_id",
    )
    .bind(id)
    .bind(auth::token_hash(&token))
    .bind(Uuid::now_v7())
    .fetch_one(&state.db)
    .await?;
    state.events.control(Scope::Session(old), Control::SessionRevoked);
    Ok(Json(BotWithToken { bot, token }))
}

/// `DELETE /bots/{id}` – Bot löschen (verlässt alle Communities; Nachrichten bleiben ohne Autor).
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    own(&state, user.user_id, id).await?;
    accounts::delete_account(&state, id, false).await?;
    tracing::info!(bot_id = %id, owner_id = %user.user_id, "Bot gelöscht");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /bots/{id}/avatar` – multipart mit Feld `file` (wie das eigene Profilbild).
pub async fn upload_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<Json<Bot>> {
    own(&state, user.user_id, id).await?;
    users::set_avatar(&state, id, multipart).await?;
    Ok(Json(by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?))
}

/// `DELETE /bots/{id}/avatar`
pub async fn delete_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Bot>> {
    own(&state, user.user_id, id).await?;
    users::clear_avatar(&state, id).await?;
    Ok(Json(by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?))
}

// --- Zu einer Community hinzufügen -----------------------------------------------

#[derive(Deserialize)]
pub struct AddBody {
    bot_id: Uuid,
}

/// `POST /communities/{id}/bots` – Bot als Mitglied aufnehmen (`MANAGE_COMMUNITY`; eigener oder öffentlicher
/// Bot). `201` mit dem Mitglied.
pub async fn add_to_community(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<AddBody>,
) -> ApiResult<(StatusCode, Json<Member>)> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;
    require_bots_enabled(&state).await?;
    let bot = by_id(&state.db, body.bot_id).await?;
    // Fremde private Bots sind nicht auffindbar.
    let bot =
        bot.filter(|b| b.owner_id == user.user_id || b.is_public).ok_or(ApiError::NotFound("unknown_bot"))?;
    let disabled: bool = sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM users WHERE id = $1")
        .bind(bot.id)
        .fetch_one(&state.db)
        .await?;
    if disabled {
        return Err(ApiError::Forbidden("account_disabled"));
    }

    let mut tx = state.db.begin().await?;
    let inserted =
        sqlx::query("INSERT INTO members (community_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(community_id)
            .bind(bot.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict("already_member"));
    }
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "bot.add",
        Some(bot.id),
        json!({ "username": bot.username }),
    )
    .await?;
    tx.commit().await?;

    invites::announce_join(&state, community_id, bot.id).await?;
    let member = models::member(&state.db, community_id, bot.id).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(member)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_routes() {
        let get = Method::GET;
        let post = Method::POST;
        let patch = Method::PATCH;
        let delete = Method::DELETE;
        // Erlaubt: eigenes Profil lesen, Nachrichten, Reaktionen, Mitglieder, Profile.
        assert!(!bot_forbidden(&get, "/auth/me"));
        assert!(!bot_forbidden(&get, "/api/auth/me"));
        assert!(!bot_forbidden(&post, "/channels/x/messages"));
        assert!(!bot_forbidden(&patch, "/messages/x"));
        assert!(!bot_forbidden(&get, "/communities"));
        assert!(!bot_forbidden(&get, "/communities/x/members"));
        assert!(!bot_forbidden(&post, "/communities/x/bots"));
        assert!(!bot_forbidden(&get, "/dms"));
        assert!(!bot_forbidden(&get, "/users/x"));
        assert!(!bot_forbidden(&post, "/webhooks/x/y"));
        // Gesperrt: Konto, Sicherheit, Freunde, DMs eröffnen, Verwaltung.
        assert!(bot_forbidden(&patch, "/auth/me"));
        assert!(bot_forbidden(&delete, "/auth/me"));
        assert!(bot_forbidden(&get, "/auth/me/export"));
        assert!(bot_forbidden(&get, "/auth/sessions"));
        assert!(bot_forbidden(&post, "/auth/password"));
        assert!(bot_forbidden(&post, "/auth/mfa/totp/setup"));
        assert!(bot_forbidden(&post, "/relationships"));
        assert!(bot_forbidden(&put_m(), "/relationships/x"));
        assert!(bot_forbidden(&post, "/dms"));
        assert!(bot_forbidden(&get, "/dms/contacts"));
        assert!(bot_forbidden(&post, "/bots"));
        assert!(bot_forbidden(&get, "/admin/users"));
        assert!(bot_forbidden(&post, "/feedback"));
        assert!(bot_forbidden(&post, "/communities"));
        assert!(bot_forbidden(&post, "/invites/abc/accept"));
        assert!(bot_forbidden(&post, "/communities/x/transfer"));
        assert!(bot_forbidden(&post, "/channels/x/voice/join"));
        assert!(bot_forbidden(&post, "/voice/state"));
        // Kein Präfix-Fehlgriff.
        assert!(!bot_forbidden(&get, "/botsfoo"));
    }

    fn put_m() -> Method {
        Method::PUT
    }

    #[test]
    fn bot_tokens_have_prefix() {
        let token = new_token();
        assert!(token.starts_with(auth::BOT_TOKEN_PREFIX));
        assert!(token.len() > 40);
    }
}
