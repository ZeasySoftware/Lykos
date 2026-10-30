//! Anmeldung, Registrierung und Verknüpfung über OpenID Connect (vom Betreiber konfigurierte Anbieter).
//!
//! Ablauf (Web): `POST …/start` liefert die Autorisierungs-URL und setzt ein kurzlebiges Cookie mit dem
//! `state` (bindet den Ablauf an diesen Browser, gegen Login-CSRF). Der Anbieter leitet auf
//! `GET …/callback` zurück; von dort geht es mit dem Ergebnis im URL-Fragment (nie an Server oder Logs)
//! zurück in den Web-Client:
//!
//! - `/` – angemeldet (Session-Cookie gesetzt)
//! - `/#mfa_token=…&mfa_methods=totp,recovery` – zweiter Faktor fehlt noch
//! - `/#oidc_signup=…` – unbekannte Identität: Benutzername wählen (`POST /auth/oidc/register`)
//! - `/#oidc_linked=<provider>` – mit dem angemeldeten Konto verknüpft
//! - `/#oidc_error=<code>` – abgebrochen oder fehlgeschlagen

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    auth::{self, AuthUser, MaybeAuthUser},
    config::OidcProviderConfig,
    error::{ApiError, ApiResult, is_unique_violation},
    mfa,
    rate_limit::ClientIp,
    routes::auth::{NewAccount, create_account, first_factor_passed, registration_mode, session_response},
    state::AppState,
};

const FLOW_COOKIE: &str = "lykos_oidc";
/// So lange darf die Anmeldung beim Anbieter dauern.
const FLOW_TTL: Duration = Duration::minutes(10);
/// So lange bleibt Zeit, nach der ersten OIDC-Anmeldung einen Benutzernamen zu wählen.
const SIGNUP_TTL: Duration = Duration::minutes(15);

fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn provider<'a>(state: &'a AppState, id: &str) -> ApiResult<&'a OidcProviderConfig> {
    state.config.oidc_providers.iter().find(|p| p.id == id).ok_or_else(ApiError::not_found)
}

fn redirect_uri(state: &AppState, provider: &str) -> String {
    format!("{}/api/auth/oidc/{provider}/callback", state.config.public_url.trim_end_matches('/'))
}

fn flow_cookie(state: &AppState, value: &str, max_age: i64) -> String {
    let secure = if state.config.public_url.starts_with("https://") { "; Secure" } else { "" };
    format!("{FLOW_COOKIE}={value}; Path=/api/auth/oidc; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.to_owned())
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Login,
    Link,
}

#[derive(Deserialize)]
pub struct StartBody {
    mode: Mode,
    session_label: Option<String>,
    /// Nur bei `link`: Bestätigung wie beim Trennen (Passwort bzw. frische Sitzung, ggf. 2FA-Code).
    #[serde(flatten)]
    reauth: mfa::Reauth,
}

#[derive(Serialize)]
pub struct StartResponse {
    url: String,
}

/// `POST /auth/oidc/{provider}/start` – Autorisierungs-URL erzeugen. `mode: link` nur angemeldet und mit
/// Re-Authentifizierung (`mfa::reauthenticate`, wie beim Trennen): Eine verknüpfte Identität ist ein
/// vollwertiger Anmeldeweg, ein gestohlenes Session-Token darf keinen eigenen dauerhaft hinzufügen.
pub async fn start(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
    ClientIp(ip): ClientIp,
    Path(provider_id): Path<String>,
    Json(body): Json<StartBody>,
) -> ApiResult<Response> {
    let provider = provider(&state, &provider_id)?;
    let link_user = match body.mode {
        Mode::Link => {
            let user = user.ok_or(ApiError::Unauthorized("unauthorized"))?;
            mfa::reauthenticate(&state, user, ip, body.reauth).await?;
            Some(user.user_id)
        }
        Mode::Login => None,
    };
    let flow_state = auth::random_token(32);
    let nonce = auth::random_token(32);
    let verifier = auth::random_token(48);
    let url = state
        .oidc
        .authorization_url(provider, &redirect_uri(&state, &provider.id), &flow_state, &nonce, &verifier)
        .await
        .map_err(|e| {
            tracing::warn!(provider = %provider.id, error = %e, "Anmeldeanbieter nicht erreichbar");
            ApiError::ServiceUnavailable("oidc_unavailable")
        })?;
    sqlx::query(
        "INSERT INTO oidc_flows (state_hash, provider, nonce, pkce_verifier, link_user_id, session_label, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(hash(&flow_state))
    .bind(&provider.id)
    .bind(&nonce)
    .bind(&verifier)
    .bind(link_user)
    .bind(body.session_label.map(|l| l.trim().chars().take(64).collect::<String>()).filter(|l| !l.is_empty()))
    .bind(OffsetDateTime::now_utc() + FLOW_TTL)
    .execute(&state.db)
    .await?;

    Ok((
        [(header::SET_COOKIE, flow_cookie(&state, &flow_state, FLOW_TTL.whole_seconds()))],
        Json(StartResponse { url }),
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Weiterleitung zurück in den Web-Client; das Ergebnis steht im Fragment.
fn back(fragment: &str, extra_cookie: Option<String>, clear_flow: String) -> Response {
    let location = if fragment.is_empty() { "/".to_owned() } else { format!("/#{fragment}") };
    let mut res = (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response();
    let headers = res.headers_mut();
    headers.append(header::SET_COOKIE, clear_flow.parse().expect("gültiger Cookie-Header"));
    if let Some(c) = extra_cookie {
        headers.append(header::SET_COOKIE, c.parse().expect("gültiger Cookie-Header"));
    }
    headers.insert(header::CACHE_CONTROL, "no-store".parse().expect("gültiger Header"));
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().expect("gültiger Header"));
    res
}

fn error_back(state: &AppState, code: &str) -> Response {
    back(&format!("oidc_error={code}"), None, flow_cookie(state, "", 0))
}

struct Flow {
    nonce: String,
    pkce_verifier: String,
    link_user_id: Option<Uuid>,
    session_label: Option<String>,
}

/// `GET /auth/oidc/{provider}/callback` – Rückkehr vom Anbieter.
pub async fn callback(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let Ok(provider) = provider(&state, &provider_id) else {
        return Ok(error_back(&state, "unknown_provider"));
    };
    if query.error.is_some() {
        return Ok(error_back(&state, "cancelled"));
    }
    let (Some(code), Some(flow_state)) = (query.code, query.state) else {
        return Ok(error_back(&state, "invalid_state"));
    };
    // Der Ablauf muss in diesem Browser gestartet worden sein (Cookie) und darf nur einmal enden.
    if cookie_value(&headers, FLOW_COOKIE).as_deref() != Some(flow_state.as_str()) {
        return Ok(error_back(&state, "invalid_state"));
    }
    let row: Option<(String, String, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "DELETE FROM oidc_flows WHERE state_hash = $1 AND provider = $2 AND expires_at > now()
         RETURNING nonce, pkce_verifier, link_user_id, session_label",
    )
    .bind(hash(&flow_state))
    .bind(&provider.id)
    .fetch_optional(&state.db)
    .await?;
    let Some((nonce, pkce_verifier, link_user_id, session_label)) = row else {
        return Ok(error_back(&state, "invalid_state"));
    };
    let flow = Flow { nonce, pkce_verifier, link_user_id, session_label };

    let claims = match state
        .oidc
        .complete(provider, &code, &flow.pkce_verifier, &redirect_uri(&state, &provider.id), &flow.nonce)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(provider = %provider.id, error = %e, "OIDC-Anmeldung fehlgeschlagen");
            return Ok(error_back(&state, "oidc_failed"));
        }
    };

    if let Some(user_id) = flow.link_user_id {
        return link(&state, provider, user_id, &claims.subject).await;
    }

    let existing: Option<Uuid> = sqlx::query_scalar(
        "UPDATE user_identities SET last_used_at = now() WHERE provider = $1 AND subject = $2 RETURNING user_id",
    )
    .bind(&provider.id)
    .bind(&claims.subject)
    .fetch_optional(&state.db)
    .await?;
    if let Some(user_id) = existing {
        return Ok(match first_factor_passed(&state, user_id, flow.session_label).await {
            // Gesperrtes Konto: keine Session, keine Challenge.
            Err(ApiError::Forbidden("account_disabled")) => error_back(&state, "account_disabled"),
            Err(e) => return Err(e),
            Ok(Ok(token)) => back("", Some(auth::session_cookie(&state, &token)), flow_cookie(&state, "", 0)),
            Ok(Err(challenge)) => back(
                &format!("mfa_token={}&mfa_methods={}", challenge.mfa_token, challenge.methods.join(",")),
                None,
                flow_cookie(&state, "", 0),
            ),
        });
    }

    // Unbekannte Identität: Registrierung nach denselben Regeln wie sonst.
    let bootstrap: bool =
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM users)").fetch_one(&state.db).await?;
    if !bootstrap && registration_mode(&state).await? == "closed" {
        return Ok(error_back(&state, "registration_closed"));
    }
    let signup = auth::random_token(32);
    sqlx::query(
        "INSERT INTO oidc_signups (token_hash, provider, subject, email, session_label, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(hash(&signup))
    .bind(&provider.id)
    .bind(&claims.subject)
    .bind(&claims.verified_email)
    .bind(&flow.session_label)
    .bind(OffsetDateTime::now_utc() + SIGNUP_TTL)
    .execute(&state.db)
    .await?;
    Ok(back(&format!("oidc_signup={signup}"), None, flow_cookie(&state, "", 0)))
}

async fn link(
    state: &AppState,
    provider: &OidcProviderConfig,
    user_id: Uuid,
    subject: &str,
) -> ApiResult<Response> {
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2")
            .bind(&provider.id)
            .bind(subject)
            .fetch_optional(&state.db)
            .await?;
    match owner {
        Some(owner) if owner == user_id => {
            return Ok(back(&format!("oidc_linked={}", provider.id), None, flow_cookie(state, "", 0)));
        }
        Some(_) => return Ok(error_back(state, "identity_in_use")),
        None => {}
    }
    let inserted =
        sqlx::query("INSERT INTO user_identities (provider, subject, user_id) VALUES ($1, $2, $3)")
            .bind(&provider.id)
            .bind(subject)
            .bind(user_id)
            .execute(&state.db)
            .await;
    match inserted {
        // (user_id, provider) ist eindeutig: höchstens eine Identität pro Anbieter und Konto.
        Err(e) if is_unique_violation(&e) => Ok(error_back(state, "already_linked")),
        Err(e) => Err(e.into()),
        Ok(_) => {
            tracing::info!(%user_id, provider = %provider.id, "OIDC-Identität verknüpft");
            Ok(back(&format!("oidc_linked={}", provider.id), None, flow_cookie(state, "", 0)))
        }
    }
}

#[derive(Deserialize)]
pub struct SignupInfoBody {
    signup_token: String,
}

#[derive(Serialize)]
pub struct SignupInfo {
    provider: String,
    provider_label: String,
    /// Vom Anbieter bestätigte Adresse, die (nach Zustimmung) übernommen werden kann.
    email: Option<String>,
}

/// `POST /auth/oidc/signup` – Angaben zur ausstehenden OIDC-Registrierung.
pub async fn signup_info(
    State(state): State<AppState>,
    Json(body): Json<SignupInfoBody>,
) -> ApiResult<Json<SignupInfo>> {
    let (provider_id, email): (String, Option<String>) = sqlx::query_as(
        "SELECT provider, email FROM oidc_signups WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(hash(&body.signup_token))
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::BadRequest("invalid_token"))?;
    let provider = provider(&state, &provider_id)?;
    Ok(Json(SignupInfo { provider: provider.id.clone(), provider_label: provider.label.clone(), email }))
}

#[derive(Deserialize)]
pub struct OidcRegisterBody {
    signup_token: String,
    username: String,
    display_name: Option<String>,
    invite: Option<String>,
    /// Bestätigte Adresse des Anbieters übernehmen (Standard: nein).
    #[serde(default)]
    use_email: bool,
}

/// `POST /auth/oidc/register` – Konto für eine neue OIDC-Identität anlegen (gleiche Regeln wie
/// `POST /auth/register`, aber ohne Passwort).
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    web: super::auth::WebClient,
    Json(body): Json<OidcRegisterBody>,
) -> ApiResult<Response> {
    let username = body.username.trim().to_owned();
    auth::validate_username(&username)?;
    let display_name = auth::normalize_display_name(body.display_name)?;
    let row: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT provider, subject, email, session_label FROM oidc_signups
         WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(hash(&body.signup_token))
    .fetch_optional(&state.db)
    .await?;
    let Some((provider, subject, email, session_label)) = row else {
        return Err(ApiError::BadRequest("invalid_token"));
    };
    state.rate_limit.register_attempt(ip)?;

    let (user_id, token) = create_account(
        &state,
        NewAccount {
            username,
            password_hash: None,
            display_name,
            invite: body.invite,
            session_label,
            verified_email: email.filter(|_| body.use_email),
            identity: Some((provider, subject)),
        },
    )
    .await?;
    // Erst nach Erfolg verbrauchen – ein vergebener Benutzername lässt sich so korrigieren.
    sqlx::query("DELETE FROM oidc_signups WHERE token_hash = $1")
        .bind(hash(&body.signup_token))
        .execute(&state.db)
        .await?;
    session_response(&state, web, StatusCode::CREATED, user_id, token).await
}

/// `DELETE /auth/identities/{provider}` – Verknüpfung lösen. Nur, wenn danach noch ein Anmeldeweg bleibt
/// (Passwort oder eine andere Identität), sonst `409 last_login_method`.
pub async fn unlink(
    State(state): State<AppState>,
    user: AuthUser,
    ClientIp(ip): ClientIp,
    Path(provider_id): Path<String>,
    Json(reauth): Json<mfa::Reauth>,
) -> ApiResult<StatusCode> {
    let (has_password, identities): (bool, i64) = sqlx::query_as(
        "SELECT u.password_hash IS NOT NULL, (SELECT count(*) FROM user_identities i WHERE i.user_id = u.id)
         FROM users u WHERE u.id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&state.db)
    .await?;
    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_identities WHERE user_id = $1 AND provider = $2)",
    )
    .bind(user.user_id)
    .bind(&provider_id)
    .fetch_one(&state.db)
    .await?;
    if !linked {
        return Err(ApiError::not_found());
    }
    if !has_password && identities <= 1 {
        return Err(ApiError::Conflict("last_login_method"));
    }
    mfa::reauthenticate(&state, user, ip, reauth).await?;
    sqlx::query("DELETE FROM user_identities WHERE user_id = $1 AND provider = $2")
        .bind(user.user_id)
        .bind(&provider_id)
        .execute(&state.db)
        .await?;
    tracing::info!(user_id = %user.user_id, provider = %provider_id, "OIDC-Identität getrennt");
    Ok(StatusCode::NO_CONTENT)
}
