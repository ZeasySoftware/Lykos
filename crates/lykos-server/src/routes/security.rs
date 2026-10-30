//! Konto-Sicherheit (Paket G): optionale E-Mail-Adresse, Passwort vergessen, 2FA-Verwaltung.
//!
//! Grundsätze: Alles ist freiwillig. Antworten verraten nie, ob ein Konto oder eine Adresse existiert.
//! E-Mail-Adressen und Codes landen nicht im Log.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    auth::{self, AuthUser},
    error::{ApiError, ApiResult, is_unique_violation},
    mail, mfa,
    rate_limit::ClientIp,
    state::AppState,
};

/// Gültigkeit eines Bestätigungslinks für eine neue Adresse.
const VERIFY_TTL: Duration = Duration::hours(24);
/// Gültigkeit eines Reset-Links.
pub const RESET_TTL: Duration = Duration::minutes(30);

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn require_mail(state: &AppState) -> ApiResult<()> {
    if state.mailer.enabled() { Ok(()) } else { Err(ApiError::BadRequest("email_unavailable")) }
}

// --- Überblick ----------------------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
pub struct IdentityInfo {
    provider: String,
    /// Beschriftung laut aktueller Konfiguration (`null`, wenn der Anbieter nicht mehr konfiguriert ist).
    #[sqlx(skip)]
    label: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    last_used_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
pub struct SecurityOverview {
    /// Bestätigte Adresse.
    email: Option<String>,
    /// Adresse, deren Bestätigung noch aussteht.
    pending_email: Option<String>,
    /// Hat das Konto ein Passwort? (Nur-OIDC-Konten nicht.)
    has_password: bool,
    mfa: mfa::MfaStatus,
    identities: Vec<IdentityInfo>,
}

pub(crate) async fn identities(state: &AppState, user_id: Uuid) -> ApiResult<Vec<IdentityInfo>> {
    let mut list: Vec<IdentityInfo> = sqlx::query_as(
        "SELECT provider, created_at, last_used_at FROM user_identities WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    for i in &mut list {
        i.label = state.config.oidc_providers.iter().find(|p| p.id == i.provider).map(|p| p.label.clone());
    }
    Ok(list)
}

/// `GET /auth/security`
pub async fn overview(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<SecurityOverview>> {
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
    Ok(Json(SecurityOverview {
        email,
        pending_email,
        has_password,
        mfa: mfa::status(&state.db, user.user_id).await?,
        identities: identities(&state, user.user_id).await?,
    }))
}

// --- E-Mail-Adresse -----------------------------------------------------------------------------------

/// Legt eine ausstehende Bestätigung an und verschickt den Link – außer die Adresse ist schon von einem
/// anderen Konto bestätigt; dann passiert nach außen dasselbe, nur ohne Mail (verrät nichts).
pub(crate) async fn start_email_verification(
    state: &AppState,
    user_id: Uuid,
    username: &str,
    email: &str,
) -> ApiResult<()> {
    state.rate_limit.mail_to_account(user_id)?;
    let token = auth::random_token(32);
    sqlx::query(
        "INSERT INTO email_verifications (user_id, email, token_hash, expires_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE SET email = $2, token_hash = $3, expires_at = $4, created_at = now()",
    )
    .bind(user_id)
    .bind(email)
    .bind(token_hash(&token))
    .bind(OffsetDateTime::now_utc() + VERIFY_TTL)
    .execute(&state.db)
    .await?;
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = lower($1) AND id <> $2)")
            .bind(email)
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    if !taken {
        // Token im Fragment: geht nicht an Server, Proxy-Logs oder per `Referer` weiter (Client liest es aus).
        let link = format!("{}/verify-email#token={token}", state.config.public_url.trim_end_matches('/'));
        state.mailer.send(mail::verify_email(
            email,
            mail::Instance::from(&*state.config),
            username,
            &link,
            VERIFY_TTL,
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct SetEmailBody {
    email: String,
    #[serde(flatten)]
    reauth: mfa::Reauth,
}

#[derive(Serialize)]
pub struct PendingEmail {
    pending_email: String,
}

/// `PUT /auth/email` – Adresse hinterlegen oder ändern. Aktiv wird sie erst mit dem Bestätigungslink.
pub async fn set_email(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(body): Json<SetEmailBody>,
) -> ApiResult<(StatusCode, Json<PendingEmail>)> {
    require_mail(&state)?;
    let email = body.email.trim().to_owned();
    if !mail::is_valid_email(&email) {
        return Err(ApiError::BadRequest("invalid_email"));
    }
    mfa::reauthenticate(&state, user, ip, body.reauth).await?;
    let (username, current): (String, Option<String>) =
        sqlx::query_as("SELECT username, email FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&state.db)
            .await?;
    if current.is_some_and(|c| c.eq_ignore_ascii_case(&email)) {
        return Err(ApiError::BadRequest("email_unchanged"));
    }
    start_email_verification(&state, user.user_id, &username, &email).await?;
    Ok((StatusCode::ACCEPTED, Json(PendingEmail { pending_email: email })))
}

/// `DELETE /auth/email` – bestätigte und ausstehende Adresse entfernen.
/// Solange 2FA per E-Mail aktiv ist: `409 email_mfa_active`.
pub async fn delete_email(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<StatusCode> {
    if mfa::status(&state.db, user.user_id).await?.email {
        return Err(ApiError::Conflict("email_mfa_active"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let mut tx = state.db.begin().await?;
    let (username, old): (String, Option<String>) =
        sqlx::query_as("SELECT username, email FROM users WHERE id = $1 FOR UPDATE")
            .bind(user.user_id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("UPDATE users SET email = NULL WHERE id = $1").bind(user.user_id).execute(&mut *tx).await?;
    for query in [
        "DELETE FROM email_verifications WHERE user_id = $1",
        "DELETE FROM password_reset_tokens WHERE user_id = $1 AND kind = 'email_reset' AND used_at IS NULL",
        "DELETE FROM mfa_email_codes WHERE user_id = $1",
    ] {
        sqlx::query(query).bind(user.user_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if let Some(old) = old {
        state.mailer.send(mail::email_removed(&old, mail::Instance::from(&*state.config), &username));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
}

#[derive(Serialize)]
pub struct VerifiedEmail {
    email: String,
}

/// `POST /auth/email/verify` – Bestätigungslink einlösen (ohne Anmeldung; das Token genügt).
pub async fn verify_email(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<VerifiedEmail>> {
    let mut tx = state.db.begin().await?;
    let (user_id, email): (Uuid, String) = sqlx::query_as(
        "DELETE FROM email_verifications WHERE token_hash = $1 AND expires_at > now() RETURNING user_id, email",
    )
    .bind(token_hash(body.token.trim()))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::BadRequest("invalid_token"))?;
    let (username, old): (String, Option<String>) =
        sqlx::query_as("SELECT username, email FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    let updated = sqlx::query("UPDATE users SET email = $2 WHERE id = $1")
        .bind(user_id)
        .bind(&email)
        .execute(&mut *tx)
        .await;
    match updated {
        // Wer den Link hat, besitzt die Adresse – ihm darf man sagen, dass sie vergeben ist.
        Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("email_taken")),
        other => other?,
    };
    // Alte Reset-Links und E-Mail-Codes gingen an die alte Adresse.
    for query in [
        "DELETE FROM password_reset_tokens WHERE user_id = $1 AND kind = 'email_reset' AND used_at IS NULL",
        "DELETE FROM mfa_email_codes WHERE user_id = $1",
    ] {
        sqlx::query(query).bind(user_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if let Some(old) = old.filter(|o| !o.eq_ignore_ascii_case(&email)) {
        state.mailer.send(mail::email_changed(&old, mail::Instance::from(&*state.config), &username, &email));
    }
    tracing::info!(%user_id, "E-Mail-Adresse bestätigt");
    Ok(Json(VerifiedEmail { email }))
}

// --- Passwort vergessen -------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ForgotBody {
    /// Benutzername oder bestätigte E-Mail-Adresse.
    login: String,
}

/// `POST /auth/password/forgot` – antwortet immer gleich (`202`), egal ob es ein passendes Konto gibt.
/// Suche, Token und Mail laufen erst nach der Antwort im Hintergrund – sonst verriete die Antwortzeit,
/// ob es das Konto gibt.
pub async fn forgot_password(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<ForgotBody>,
) -> ApiResult<StatusCode> {
    require_mail(&state)?;
    let login = body.login.trim().to_owned();
    if login.is_empty() || login.len() > mail::EMAIL_MAX_LEN {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    state.rate_limit.reset_request(ip)?;
    tokio::spawn(async move {
        if let Err(e) = send_reset_link(&state, &login).await {
            tracing::warn!(error = ?e, "Reset-Link konnte nicht verschickt werden");
        }
    });
    Ok(StatusCode::ACCEPTED)
}

/// Hintergrundteil von `forgot_password`: passendes Konto suchen, Link anlegen, Mail verschicken.
async fn send_reset_link(state: &AppState, login: &str) -> ApiResult<()> {
    // Gesperrte Konten bekommen still keinen Link (Antwort bleibt gleich).
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, username, email FROM users
         WHERE email IS NOT NULL AND disabled_at IS NULL
           AND (lower(username) = lower($1) OR lower(email) = lower($1))
         LIMIT 1",
    )
    .bind(login)
    .fetch_optional(&state.db)
    .await?;
    if let Some((user_id, username, email)) = row
        && state.rate_limit.reset_account(user_id)
    {
        // Gemeinsame Tabelle mit den Admin-Links (Paket H); eingelöst über `/auth/password-reset/*`.
        // Ein neuer Link ersetzt ältere, noch offene E-Mail-Links.
        let token = auth::random_token(32);
        let mut tx = state.db.begin().await?;
        sqlx::query(
            "DELETE FROM password_reset_tokens WHERE user_id = $1 AND kind = 'email_reset' AND used_at IS NULL",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO password_reset_tokens (id, user_id, token_hash, kind, expires_at)
             VALUES ($1, $2, $3, 'email_reset', $4)",
        )
        .bind(Uuid::now_v7())
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(OffsetDateTime::now_utc() + RESET_TTL)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let link = format!("{}/reset-password#token={token}", state.config.public_url.trim_end_matches('/'));
        state.mailer.send(mail::password_reset(
            &email,
            mail::Instance::from(&*state.config),
            &username,
            &link,
            RESET_TTL,
        ));
    }
    Ok(())
}

// --- Zwei-Faktor: Verwaltung --------------------------------------------------------------------------

#[derive(Serialize)]
pub struct TotpSetup {
    /// Base32-Geheimnis zum Abtippen.
    secret: String,
    otpauth_uri: String,
    /// QR-Code als SVG-Dokument.
    qr_svg: String,
}

/// `POST /auth/mfa/totp/setup` – neues Geheimnis erzeugen (noch nicht aktiv).
pub async fn totp_setup(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<Json<TotpSetup>> {
    if mfa::status(&state.db, user.user_id).await?.totp {
        return Err(ApiError::Conflict("totp_already_enabled"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    let secret = mfa::new_totp_secret();
    sqlx::query(
        "INSERT INTO user_mfa (user_id, totp_pending_secret) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET totp_pending_secret = $2, updated_at = now()",
    )
    .bind(user.user_id)
    .bind(&secret)
    .execute(&state.db)
    .await?;
    let issuer = if state.config.instance_name == "Lykos" {
        "Lykos".to_owned()
    } else {
        format!("Lykos ({})", state.config.instance_name)
    };
    let otpauth_uri = mfa::otpauth_uri(&issuer, &username, &secret);
    Ok(Json(TotpSetup { secret: mfa::base32(&secret), qr_svg: mfa::qr_svg(&otpauth_uri)?, otpauth_uri }))
}

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

#[derive(Serialize)]
pub struct RecoveryCodes {
    /// Nur bei der ersten 2FA-Methode bzw. beim Neuerzeugen; sonst `null`.
    recovery_codes: Option<Vec<String>>,
}

/// Nach dem Einschalten einer Methode: Wiederherstellungscodes erzeugen, falls es noch keine 2FA gab.
async fn codes_if_first(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    was_active: bool,
) -> ApiResult<RecoveryCodes> {
    Ok(RecoveryCodes {
        recovery_codes: if was_active { None } else { Some(mfa::replace_recovery_codes(tx, user_id).await?) },
    })
}

/// `POST /auth/mfa/totp/enable` – Einrichtung mit dem ersten Code abschließen.
pub async fn totp_enable(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CodeBody>,
) -> ApiResult<Json<RecoveryCodes>> {
    let before = mfa::status(&state.db, user.user_id).await?;
    if before.totp {
        return Err(ApiError::Conflict("totp_already_enabled"));
    }
    let pending: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT totp_pending_secret FROM user_mfa WHERE user_id = $1")
            .bind(user.user_id)
            .fetch_optional(&state.db)
            .await?
            .flatten();
    let Some(secret) = pending else { return Err(ApiError::BadRequest("totp_not_started")) };
    let digits: String = body.code.chars().filter(|c| !c.is_whitespace()).collect();
    let now = mfa::current_step();
    let step = digits
        .parse::<u32>()
        .ok()
        .filter(|_| digits.len() == 6)
        .and_then(|value| (now - 1..=now + 1).find(|s| mfa::totp_at(&secret, *s) == value));
    let Some(step) = step else { return Err(ApiError::BadRequest("invalid_code")) };

    let mut tx = state.db.begin().await?;
    let updated = sqlx::query(
        "UPDATE user_mfa SET totp_secret = totp_pending_secret, totp_pending_secret = NULL, totp_last_step = $2,
                updated_at = now()
         WHERE user_id = $1 AND totp_pending_secret = $3",
    )
    .bind(user.user_id)
    .bind(step)
    .bind(&secret)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated != 1 {
        return Err(ApiError::BadRequest("totp_not_started"));
    }
    let codes = codes_if_first(&mut tx, user.user_id, before.active()).await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.user_id, "2FA: Authenticator-App aktiviert");
    Ok(Json(codes))
}

/// Entfernt die Wiederherstellungscodes, wenn keine Methode mehr aktiv ist.
async fn cleanup_if_inactive(tx: &mut sqlx::PgConnection, user_id: Uuid) -> ApiResult<()> {
    sqlx::query(
        "DELETE FROM mfa_recovery_codes WHERE user_id = $1 AND NOT EXISTS (
             SELECT 1 FROM user_mfa WHERE user_id = $1 AND (totp_secret IS NOT NULL OR email_enabled))",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// `DELETE /auth/mfa/totp` – Authenticator-App entfernen.
pub async fn totp_disable(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<StatusCode> {
    if !mfa::status(&state.db, user.user_id).await?.totp {
        return Err(ApiError::Conflict("totp_not_enabled"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE user_mfa SET totp_secret = NULL, totp_pending_secret = NULL, totp_last_step = NULL, updated_at = now()
         WHERE user_id = $1",
    )
    .bind(user.user_id)
    .execute(&mut *tx)
    .await?;
    cleanup_if_inactive(&mut tx, user.user_id).await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.user_id, "2FA: Authenticator-App entfernt");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/mfa/email` – 2FA per E-Mail einschalten (nur mit bestätigter Adresse).
pub async fn email_mfa_enable(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<Json<RecoveryCodes>> {
    require_mail(&state)?;
    let has_email: bool = sqlx::query_scalar("SELECT email IS NOT NULL FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    if !has_email {
        return Err(ApiError::BadRequest("email_unavailable"));
    }
    let before = mfa::status(&state.db, user.user_id).await?;
    if before.email {
        return Err(ApiError::Conflict("email_mfa_already_enabled"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO user_mfa (user_id, email_enabled) VALUES ($1, true)
         ON CONFLICT (user_id) DO UPDATE SET email_enabled = true, updated_at = now()",
    )
    .bind(user.user_id)
    .execute(&mut *tx)
    .await?;
    let codes = codes_if_first(&mut tx, user.user_id, before.active()).await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.user_id, "2FA: E-Mail-Codes aktiviert");
    Ok(Json(codes))
}

/// `DELETE /auth/mfa/email` – 2FA per E-Mail ausschalten.
pub async fn email_mfa_disable(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<StatusCode> {
    if !mfa::status(&state.db, user.user_id).await?.email {
        return Err(ApiError::Conflict("email_mfa_not_enabled"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE user_mfa SET email_enabled = false, updated_at = now() WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM mfa_email_codes WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    cleanup_if_inactive(&mut tx, user.user_id).await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.user_id, "2FA: E-Mail-Codes deaktiviert");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/mfa/recovery-codes` – zehn neue Wiederherstellungscodes (alte werden ungültig).
pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<Json<RecoveryCodes>> {
    if !mfa::status(&state.db, user.user_id).await?.active() {
        return Err(ApiError::Conflict("mfa_not_enabled"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    let mut tx = state.db.begin().await?;
    let codes = mfa::replace_recovery_codes(&mut tx, user.user_id).await?;
    tx.commit().await?;
    Ok(Json(RecoveryCodes { recovery_codes: Some(codes) }))
}

/// `POST /auth/mfa/email-code` – Code per E-Mail anfordern, um eine sensible Aktion zu bestätigen
/// (nur bei aktiver 2FA per E-Mail).
pub async fn send_reauth_code(State(state): State<AppState>, user: AuthUser) -> ApiResult<StatusCode> {
    if !mfa::status(&state.db, user.user_id).await?.email {
        return Err(ApiError::BadRequest("email_unavailable"));
    }
    mfa::send_email_code(&state, user.user_id).await?;
    Ok(StatusCode::ACCEPTED)
}
