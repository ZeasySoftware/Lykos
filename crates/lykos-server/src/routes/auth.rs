use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    accounts, attachments,
    auth::{self, AuthUser},
    error::{ApiError, ApiResult, is_unique_violation},
    events::{Control, Scope},
    gateway, instance_settings, mail, mfa,
    models::{self, User, double_option},
    rate_limit::ClientIp,
    routes::{customization, feedback, invites, relationships, security, users},
    state::AppState,
};

#[derive(Serialize)]
pub struct AuthResponse {
    /// Fehlt für den eigenen Web-Client: Dort trägt allein das HttpOnly-Cookie die Sitzung, und ein
    /// Token im Body wäre bei einer XSS-Lücke lesbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    user: User,
}

/// Kommt die Anfrage vom eigenen Web-Client (`Origin` = `public_url`)? Desktop-App, Dev-Client
/// (andere Origin) und Werkzeuge ohne `Origin` brauchen das Token im Body (Bearer-Betrieb).
#[derive(Debug, Clone, Copy)]
pub struct WebClient(pub bool);

impl FromRequestParts<AppState> for WebClient {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let origin = parts.headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let own = gateway::normalize_origin(&state.config.public_url);
        Ok(Self(origin.is_some_and(|o| gateway::normalize_origin(o) == own)))
    }
}

/// Erfolgreiche Anmeldung: Cookie immer, Token im Body nur für Clients außerhalb des eigenen Web-Clients.
pub(crate) async fn session_response(
    state: &AppState,
    web: WebClient,
    status: StatusCode,
    user_id: Uuid,
    token: String,
) -> ApiResult<Response> {
    let user = models::user_by_id(&state.db, user_id).await?.ok_or_else(ApiError::not_found)?;
    let cookie = auth::session_cookie(state, &token);
    let body = AuthResponse { token: (!web.0).then_some(token), user };
    Ok((status, [(header::SET_COOKIE, cookie)], Json(body)).into_response())
}

#[derive(Deserialize)]
pub struct RegisterBody {
    username: String,
    password: String,
    display_name: Option<String>,
    invite: Option<String>,
    session_label: Option<String>,
    /// Optional, nur zur Kontowiederherstellung; wird erst nach Bestätigung per Link aktiv.
    email: Option<String>,
}

/// Ein neues Konto – per Passwort oder per OIDC-Identität.
pub(crate) struct NewAccount {
    pub username: String,
    pub password_hash: Option<String>,
    pub display_name: Option<String>,
    pub invite: Option<String>,
    pub session_label: Option<String>,
    /// Vom OIDC-Anbieter bestätigte Adresse (nur nach Zustimmung).
    pub verified_email: Option<String>,
    /// `(provider, subject)` der OIDC-Identität.
    pub identity: Option<(String, String)>,
}

/// Legt Konto und Session an (gleiche Regeln für alle Wege: Registrierungsmodus, Einladung, erster Nutzer
/// wird Instanz-Admin). Gibt `(user_id, token)` zurück.
pub(crate) async fn create_account(state: &AppState, account: NewAccount) -> ApiResult<(Uuid, String)> {
    let mode = registration_mode(state).await?;
    let mut tx = state.db.begin().await?;
    // Serialisiert Registrierungen, damit genau ein „erster Nutzer“ Admin wird.
    sqlx::query("SELECT pg_advisory_xact_lock(7001)").execute(&mut *tx).await?;

    let bootstrap: bool =
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM users)").fetch_one(&mut *tx).await?;
    let invite = account.invite.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());

    if !bootstrap {
        match mode.as_str() {
            "closed" => return Err(ApiError::Forbidden("registration_closed")),
            "invite_only" if invite.is_none() => return Err(ApiError::Forbidden("invite_required")),
            _ => {}
        }
    }

    // Eine schon anderweitig bestätigte Adresse wird still nicht übernommen (verrät nichts).
    let email = match account.verified_email {
        Some(email) => {
            let taken: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = lower($1))")
                    .bind(&email)
                    .fetch_one(&mut *tx)
                    .await?;
            (!taken).then_some(email)
        }
        None => None,
    };

    let user_id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_instance_admin, email)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(&account.username)
    .bind(&account.display_name)
    .bind(&account.password_hash)
    .bind(bootstrap)
    .bind(&email)
    .execute(&mut *tx)
    .await;
    match inserted {
        Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("username_taken")),
        other => other?,
    };

    if let Some((provider, subject)) = &account.identity {
        let linked = sqlx::query(
            "INSERT INTO user_identities (provider, subject, user_id, last_used_at) VALUES ($1, $2, $3, now())",
        )
        .bind(provider)
        .bind(subject)
        .bind(user_id)
        .execute(&mut *tx)
        .await;
        match linked {
            Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("identity_in_use")),
            other => other?,
        };
    }

    let joined = match &invite {
        // Community- oder Instanz-Einladung (Paket H); gilt für Passwort- und OIDC-Registrierung.
        Some(code) => invites::redeem_for_registration(&mut tx, code, user_id).await?,
        None => None,
    };

    let token = auth::create_session(&mut *tx, user_id, account.session_label).await?;
    tx.commit().await?;

    if let Some(community_id) = joined {
        invites::announce_join(state, community_id, user_id).await?;
    }
    if bootstrap {
        tracing::info!(username = %account.username, "erster Nutzer registriert – Instanz-Admin");
    }
    Ok((user_id, token))
}

/// Registrierungsmodus der Instanz (`open`, `invite_only`, `closed`) – Laufzeit-Einstellung der
/// Instanz-Verwaltung, sonst Standard.
pub async fn registration_mode(state: &AppState) -> ApiResult<String> {
    Ok(instance_settings::load(&state.db, &state.config).await?.registration_mode)
}

pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    web: WebClient,
    Json(body): Json<RegisterBody>,
) -> ApiResult<impl IntoResponse> {
    let username = body.username.trim().to_owned();
    auth::validate_username(&username)?;
    auth::validate_password(&body.password)?;
    let display_name = auth::normalize_display_name(body.display_name)?;
    let email = body.email.map(|e| e.trim().to_owned()).filter(|e| !e.is_empty());
    if let Some(email) = &email {
        if !state.mailer.enabled() {
            return Err(ApiError::BadRequest("email_unavailable"));
        }
        if !mail::is_valid_email(email) {
            return Err(ApiError::BadRequest("invalid_email"));
        }
    }
    // Gezählt wird jeder formal gültige Versuch (auch abgelehnte), vor dem teuren Hashing.
    state.rate_limit.register_attempt(ip)?;

    // Teuer, daher vor der Transaktion.
    let password_hash = auth::hash_password(body.password).await?;
    let (user_id, token) = create_account(
        &state,
        NewAccount {
            username: username.clone(),
            password_hash: Some(password_hash),
            display_name,
            invite: body.invite,
            session_label: body.session_label,
            verified_email: None,
            identity: None,
        },
    )
    .await?;

    if let Some(email) = email {
        // Fehler hier (z. B. Mail-Limit) sollen die Registrierung nicht scheitern lassen.
        if let Err(e) = security::start_email_verification(&state, user_id, &username, &email).await {
            tracing::warn!(error = %e, "Bestätigungsmail bei Registrierung nicht angestoßen");
        }
    }
    session_response(&state, web, StatusCode::CREATED, user_id, token).await
}

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
    session_label: Option<String>,
}

/// Zweiter Anmeldeschritt nötig (2FA aktiv): statt einer Session gibt es ein kurzlebiges `mfa_token`.
#[derive(Serialize)]
pub(crate) struct MfaRequired {
    mfa_required: bool,
    pub(crate) mfa_token: String,
    pub(crate) methods: Vec<&'static str>,
}

/// Nach korrektem erstem Faktor: Session anlegen – oder, bei aktiver 2FA, eine Challenge.
pub(crate) async fn first_factor_passed(
    state: &AppState,
    user_id: Uuid,
    session_label: Option<String>,
) -> ApiResult<Result<String, MfaRequired>> {
    // Gesperrte Konten kommen nicht bis zur 2FA-Challenge.
    auth::ensure_active(&state.db, user_id).await?;
    let status = mfa::status(&state.db, user_id).await?;
    if !status.active() {
        return Ok(Ok(auth::create_session(&state.db, user_id, session_label).await?));
    }
    let has_email: bool = sqlx::query_scalar("SELECT email IS NOT NULL FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    let mfa_token = mfa::create_challenge(&state.db, user_id, session_label).await?;
    Ok(Err(MfaRequired {
        mfa_required: true,
        mfa_token,
        methods: mfa::methods(&status, has_email && state.mailer.enabled()),
    }))
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    web: WebClient,
    Json(body): Json<LoginBody>,
) -> ApiResult<Response> {
    // Gesperrt wird vor der Passwortprüfung – auch ein richtiges Passwort hilft dann nicht.
    state.rate_limit.check_login(&body.username, ip)?;

    let row: Option<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE lower(username) = lower($1)")
            .bind(body.username.trim())
            .fetch_optional(&state.db)
            .await?;

    // Konten ohne Passwort (nur OIDC) werden wie unbekannte Nutzer gegen den Dummy-Hash geprüft.
    let (user_id, hash) = row.map_or((None, None), |(id, h)| (Some(id), h));
    let valid = auth::verify_password(body.password, hash).await?;
    let (true, Some(user_id)) = (valid, user_id) else {
        state.rate_limit.login_failed(&body.username, ip);
        return Err(ApiError::Unauthorized("invalid_credentials"));
    };

    match first_factor_passed(&state, user_id, body.session_label).await? {
        Ok(token) => {
            state.rate_limit.login_succeeded(&body.username, ip);
            session_response(&state, web, StatusCode::OK, user_id, token).await
        }
        // Der Zähler des Benutzernamens bleibt, bis auch der zweite Faktor stimmt.
        Err(challenge) => Ok(Json(challenge).into_response()),
    }
}

#[derive(Deserialize)]
pub struct LoginMfaBody {
    mfa_token: String,
    method: mfa::Method,
    code: String,
}

/// `POST /auth/login/mfa` – zweiter Anmeldeschritt.
pub async fn login_mfa(
    State(state): State<AppState>,
    web: WebClient,
    ClientIp(ip): ClientIp,
    Json(body): Json<LoginMfaBody>,
) -> ApiResult<Response> {
    let user_id = mfa::challenge_user(&state.db, &body.mfa_token).await?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    state.rate_limit.check_login(&username, ip)?;
    auth::ensure_active(&state.db, user_id).await?;
    // Zählt den Versuch an der Challenge (höchstens 5, dann ist das Token verbraucht).
    let (user_id, session_label) = mfa::use_challenge(&state.db, &body.mfa_token).await?;

    let status = mfa::status(&state.db, user_id).await?;
    let allowed = match body.method {
        mfa::Method::Totp => status.totp,
        mfa::Method::Email => status.email,
        mfa::Method::Recovery => true,
    };
    if !allowed || !mfa::verify(&state, user_id, body.method, &body.code).await? {
        state.rate_limit.login_failed(&username, ip);
        return Err(ApiError::Unauthorized("invalid_code"));
    }
    state.rate_limit.login_succeeded(&username, ip);
    mfa::delete_challenge(&state.db, &body.mfa_token).await?;
    let token = auth::create_session(&state.db, user_id, session_label).await?;
    session_response(&state, web, StatusCode::OK, user_id, token).await
}

#[derive(Deserialize)]
pub struct MfaTokenBody {
    mfa_token: String,
}

/// `POST /auth/login/mfa/email` – Code für den zweiten Anmeldeschritt per E-Mail anfordern.
pub async fn login_mfa_email(
    State(state): State<AppState>,
    Json(body): Json<MfaTokenBody>,
) -> ApiResult<StatusCode> {
    let user_id = mfa::challenge_user(&state.db, &body.mfa_token).await?;
    if !mfa::status(&state.db, user_id).await?.email {
        return Err(ApiError::BadRequest("email_unavailable"));
    }
    mfa::send_email_code(&state, user_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn logout(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    revoke(&state, user.user_id, user.session_id).await?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, auth::clear_cookie())]))
}

pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<User>> {
    let user =
        models::user_by_id(&state.db, user.user_id).await?.ok_or(ApiError::Unauthorized("unauthorized"))?;
    Ok(Json(user))
}

#[derive(Deserialize)]
pub struct UpdateMeBody {
    #[serde(default, deserialize_with = "double_option")]
    display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    bio: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    status_text: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pronouns: Option<Option<String>>,
    /// 0xRRGGBB; gilt, wenn kein Bannerbild gesetzt ist.
    #[serde(default, deserialize_with = "double_option")]
    banner_color: Option<Option<i32>>,
    /// 0xRRGGBB; Rahmen/Verlauf der Profilkarte.
    #[serde(default, deserialize_with = "double_option")]
    accent_color: Option<Option<i32>>,
}

pub async fn update_me(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<UpdateMeBody>,
) -> ApiResult<Json<User>> {
    // Erst alles prüfen, dann schreiben – eine ungültige Angabe ändert nichts.
    let display_name = body.display_name.map(auth::normalize_display_name).transpose()?;
    let bio = body.bio.map(|b| auth::normalize_text(b, auth::BIO_MAX_CHARS, "invalid_bio")).transpose()?;
    let status_text = body
        .status_text
        .map(|t| auth::normalize_text(t, auth::STATUS_TEXT_MAX_CHARS, "invalid_status_text"))
        .transpose()?;
    let pronouns = body
        .pronouns
        .map(|p| auth::normalize_text(p, customization::PRONOUNS_MAX_CHARS, "invalid_pronouns"))
        .transpose()?;
    if pronouns.as_ref().is_some_and(|p| p.as_deref().is_some_and(|p| p.contains('\n'))) {
        return Err(ApiError::BadRequest("invalid_pronouns"));
    }
    for color in [body.banner_color, body.accent_color].into_iter().flatten() {
        customization::validate_color(color)?;
    }

    // `COALESCE`-frei: Feld nur setzen, wenn es im Body vorkam ($n = true).
    sqlx::query(
        "UPDATE users SET
            display_name = CASE WHEN $2 THEN $3 ELSE display_name END,
            bio          = CASE WHEN $4 THEN $5 ELSE bio END,
            status_text  = CASE WHEN $6 THEN $7 ELSE status_text END,
            pronouns     = CASE WHEN $8 THEN $9 ELSE pronouns END,
            banner_color = CASE WHEN $10 THEN $11 ELSE banner_color END,
            accent_color = CASE WHEN $12 THEN $13 ELSE accent_color END
         WHERE id = $1",
    )
    .bind(user.user_id)
    .bind(display_name.is_some())
    .bind(display_name.flatten())
    .bind(bio.is_some())
    .bind(bio.flatten())
    .bind(status_text.is_some())
    .bind(status_text.flatten())
    .bind(pronouns.is_some())
    .bind(pronouns.flatten())
    .bind(body.banner_color.is_some())
    .bind(body.banner_color.flatten())
    .bind(body.accent_color.is_some())
    .bind(body.accent_color.flatten())
    .execute(&state.db)
    .await?;

    Ok(Json(users::announce_user_update(&state, user.user_id).await?))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionInfo {
    id: Uuid,
    label: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    last_used_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    #[sqlx(default)]
    current: bool,
}

pub async fn sessions(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<SessionInfo>>> {
    Ok(Json(list_sessions(&state, user).await?))
}

async fn list_sessions(state: &AppState, user: AuthUser) -> ApiResult<Vec<SessionInfo>> {
    let mut sessions: Vec<SessionInfo> = sqlx::query_as(
        "SELECT id, label, created_at, last_used_at, expires_at FROM sessions
         WHERE user_id = $1 AND expires_at > now() ORDER BY last_used_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    for s in &mut sessions {
        s.current = s.id == user.session_id;
    }
    Ok(sessions)
}

pub async fn revoke_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(session_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if !revoke(&state, user.user_id, session_id).await? {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Löscht eine Session des Nutzers und trennt zugehörige Gateway-Verbindungen.
async fn revoke(state: &AppState, user_id: Uuid, session_id: Uuid) -> ApiResult<bool> {
    let deleted = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
        .bind(session_id)
        .bind(user_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if deleted > 0 {
        state.events.control(Scope::Session(session_id), Control::SessionRevoked);
    }
    Ok(deleted > 0)
}

/// `DELETE /auth/sessions` – alle Sessions außer der aktuellen beenden.
pub async fn revoke_other_sessions(State(state): State<AppState>, user: AuthUser) -> ApiResult<StatusCode> {
    revoke_others(&state, user).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_others(state: &AppState, user: AuthUser) -> ApiResult<()> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 AND id <> $2 RETURNING id")
            .bind(user.user_id)
            .bind(user.session_id)
            .fetch_all(&state.db)
            .await?;
    for id in ids {
        state.events.control(Scope::Session(id), Control::SessionRevoked);
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    /// Pflicht, sofern das Konto schon ein Passwort hat (OIDC-Konten setzen so ein erstes Passwort).
    #[serde(default)]
    current_password: Option<String>,
    new_password: String,
    /// Alle anderen Sessions beenden (Standard: ja).
    #[serde(default = "default_true")]
    logout_other_sessions: bool,
    /// 2FA-Code, falls 2FA aktiv ist.
    #[serde(default)]
    mfa_code: Option<String>,
    #[serde(default)]
    mfa_method: Option<mfa::Method>,
}

fn default_true() -> bool {
    true
}

/// `POST /auth/password` – Passwort ändern (Re-Authentifizierung nötig).
pub async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(body): Json<ChangePasswordBody>,
) -> ApiResult<StatusCode> {
    auth::validate_password(&body.new_password)?;
    let reauth =
        mfa::Reauth { password: body.current_password, mfa_code: body.mfa_code, mfa_method: body.mfa_method };
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let hash = auth::hash_password(body.new_password).await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user.user_id)
        .bind(hash)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM password_reset_tokens WHERE user_id = $1 AND used_at IS NULL")
        .bind(user.user_id)
        .execute(&state.db)
        .await?;
    if body.logout_other_sessions {
        revoke_others(&state, user).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- Datenexport -------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct ExportMembership {
    community_id: Uuid,
    community_name: String,
    is_owner: bool,
    nickname: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    joined_at: OffsetDateTime,
    roles: Vec<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ExportMessage {
    id: Uuid,
    /// `null` bei Direktnachrichten.
    community_id: Option<Uuid>,
    community_name: Option<String>,
    channel_id: Uuid,
    channel_name: String,
    content: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    edited_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ExportAttachment {
    id: Uuid,
    /// `null` bei Direktnachrichten.
    community_name: Option<String>,
    channel_name: String,
    message_id: Option<Uuid>,
    filename: String,
    content_type: String,
    size_bytes: i64,
    width: Option<i32>,
    height: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct ExportIdentity {
    provider: String,
    subject: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_used_at: Option<OffsetDateTime>,
}

/// `GET /auth/me/export` – alle über den Nutzer gespeicherten Daten als JSON-Download
/// (Art. 15/20 DSGVO). Anhänge nur als Metadaten; die Dateien selbst sind über ihre URLs abrufbar.
pub async fn export(State(state): State<AppState>, user: AuthUser) -> ApiResult<impl IntoResponse> {
    let me =
        models::user_by_id(&state.db, user.user_id).await?.ok_or(ApiError::Unauthorized("unauthorized"))?;
    let sessions = list_sessions(&state, user).await?;
    let memberships: Vec<ExportMembership> = sqlx::query_as(
        "SELECT c.id AS community_id, c.name AS community_name, c.owner_id = m.user_id AS is_owner,
                m.nickname, m.joined_at,
                COALESCE(array_agg(r.name ORDER BY r.position) FILTER (WHERE r.id IS NOT NULL), '{}') AS roles
         FROM members m
         JOIN communities c ON c.id = m.community_id
         LEFT JOIN member_roles mr ON mr.community_id = m.community_id AND mr.user_id = m.user_id
         LEFT JOIN roles r ON r.id = mr.role_id
         WHERE m.user_id = $1
         GROUP BY c.id, c.name, c.owner_id, m.user_id, m.nickname, m.joined_at
         ORDER BY m.joined_at",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    let messages: Vec<ExportMessage> = sqlx::query_as(
        "SELECT msg.id, co.id AS community_id, co.name AS community_name, c.id AS channel_id,
                c.name AS channel_name, msg.content, msg.created_at, msg.edited_at
         FROM messages msg
         JOIN channels c ON c.id = msg.channel_id
         LEFT JOIN communities co ON co.id = c.community_id
         WHERE msg.author_id = $1
         ORDER BY msg.id",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    let attachments: Vec<ExportAttachment> = sqlx::query_as(
        "SELECT a.id, co.name AS community_name, c.name AS channel_name, a.message_id, a.filename,
                a.content_type, a.size_bytes, a.width, a.height, a.created_at
         FROM attachments a
         JOIN channels c ON c.id = a.channel_id
         LEFT JOIN communities co ON co.id = a.community_id
         WHERE a.uploader_id = $1
         ORDER BY a.id",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;

    // Konto-Sicherheit (Paket G): Adresse, 2FA-Stand und verknüpfte Identitäten – ohne Geheimnisse.
    let (email, has_password): (Option<String>, bool) =
        sqlx::query_as("SELECT email, password_hash IS NOT NULL FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&state.db)
            .await?;
    let pending_email: Option<String> =
        sqlx::query_scalar("SELECT email FROM email_verifications WHERE user_id = $1 AND expires_at > now()")
            .bind(user.user_id)
            .fetch_optional(&state.db)
            .await?;
    let two_factor = mfa::status(&state.db, user.user_id).await?;
    let identities: Vec<ExportIdentity> = sqlx::query_as(
        "SELECT provider, subject, created_at, last_used_at FROM user_identities WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    // Eigene Fehler- und Vorschlagsmeldungen (Paket K), wie der Melder sie sieht.
    let feedback = feedback::all_of_reporter(&state, user.user_id).await?;
    // Freundesystem: eigene Freunde, Anfragen und Blockaden samt Einstellung.
    let relationships = relationships::export(&state.db, user.user_id).await?;
    let friend_requests = relationships::export_setting(&state.db, user.user_id).await?;

    let now = OffsetDateTime::now_utc();
    let body = serde_json::json!({
        "format": "lykos-export-1",
        "instance": instance_settings::load(&state.db, &state.config).await?.instance_name,
        "exported_at": now.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
        "profile": me,
        "account_security": {
            "email": email,
            "pending_email": pending_email,
            "has_password": has_password,
            "two_factor": two_factor,
            "identities": identities,
        },
        "sessions": sessions,
        "memberships": memberships,
        "messages": messages,
        "attachments": attachments,
        "feedback": feedback,
        "relationships": relationships,
        "privacy": { "friend_requests": friend_requests },
    });
    let filename = format!("lykos-export-{}-{}.json", me.username, now.date());
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (header::CONTENT_DISPOSITION, attachments::content_disposition(false, &filename)),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        serde_json::to_string_pretty(&body).map_err(anyhow::Error::from)?,
    ))
}

// --- Konto löschen -----------------------------------------------------------

#[derive(Deserialize)]
pub struct DeleteMeBody {
    /// Passwort und ggf. 2FA-Code.
    #[serde(flatten)]
    reauth: mfa::Reauth,
    /// Eigene Nachrichten mitlöschen; sonst bleiben sie als „Gelöschter Nutzer“ stehen.
    #[serde(default)]
    delete_messages: bool,
}

/// `DELETE /auth/me` – Konto endgültig löschen. Besitzer von Communities müssen diese vorher übertragen
/// oder löschen (`409 owns_communities`); der letzte Instanz-Admin kann sich nicht löschen
/// (`409 last_instance_admin`).
pub async fn delete_me(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(body): Json<DeleteMeBody>,
) -> ApiResult<impl IntoResponse> {
    mfa::reauthenticate(&state, user, ip, body.reauth).await?;
    let user_id = user.user_id;

    let owns: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM communities WHERE owner_id = $1)")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    if owns {
        return Err(ApiError::Conflict("owns_communities"));
    }
    let last_admin: bool = sqlx::query_scalar(
        "SELECT u.is_instance_admin AND NOT EXISTS (
             SELECT 1 FROM users o WHERE o.is_instance_admin AND o.id <> u.id)
         FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    if last_admin {
        return Err(ApiError::Conflict("last_instance_admin"));
    }

    accounts::delete_account(&state, user_id, body.delete_messages).await?;
    tracing::info!(%user_id, delete_messages = body.delete_messages, "Konto gelöscht");
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, auth::clear_cookie())]))
}
