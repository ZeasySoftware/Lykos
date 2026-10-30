//! Einlösen von Passwort-Reset-Links (`/reset-password#token=…` im Client; ältere Mails: `?token=…`).
//!
//! Die Tokens liegen gehasht in `password_reset_tokens`; die Einlösung ist unabhängig von `kind`
//! (`admin_reset`, 24 h, siehe `routes::admin::users::password_reset_link`; `email_reset`, 30 min, aus
//! „Passwort vergessen“, siehe `routes::security::forgot_password`). Ein Link gilt einmal:
//! Nach dem Setzen des neuen Passworts ist er verbraucht und alle Sessions des Kontos sind beendet.
//!
//! Token stehen im Request-Body, nicht in der URL – so landen sie nicht in Zugriffslogs.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    auth,
    error::{ApiError, ApiResult},
    events::{Control, Scope},
    state::AppState,
};

#[derive(Deserialize)]
pub struct VerifyBody {
    token: String,
}

#[derive(Serialize)]
pub struct ResetInfo {
    username: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

/// Link noch nicht benutzt und nicht abgelaufen (Alias `t`).
macro_rules! valid {
    () => {
        "t.used_at IS NULL AND t.expires_at > now()"
    };
}

/// `POST /auth/password-reset/verify` – prüft einen Link, bevor der Client das Formular zeigt.
pub async fn verify(
    State(state): State<AppState>,
    Json(body): Json<VerifyBody>,
) -> ApiResult<Json<ResetInfo>> {
    let row: Option<(String, OffsetDateTime, bool)> = sqlx::query_as(concat!(
        "SELECT u.username, t.expires_at, u.disabled_at IS NOT NULL
         FROM password_reset_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND ",
        valid!()
    ))
    .bind(auth::token_hash(body.token.trim()))
    .fetch_optional(&state.db)
    .await?;
    let (username, expires_at, disabled) = row.ok_or(ApiError::NotFound("invalid_reset_token"))?;
    if disabled {
        return Err(ApiError::Forbidden("account_disabled"));
    }
    Ok(Json(ResetInfo { username, expires_at }))
}

#[derive(Deserialize)]
pub struct CompleteBody {
    token: String,
    new_password: String,
}

/// `POST /auth/password-reset/complete` – neues Passwort setzen. Beendet alle Sessions; danach normal anmelden.
pub async fn complete(
    State(state): State<AppState>,
    Json(body): Json<CompleteBody>,
) -> ApiResult<StatusCode> {
    auth::validate_password(&body.new_password)?;
    let token_hash = auth::token_hash(body.token.trim());
    // Vorab prüfen, damit ungültige Links kein teures Hashing auslösen.
    let known: Option<bool> = sqlx::query_scalar(concat!(
        "SELECT u.disabled_at IS NOT NULL FROM password_reset_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND ",
        valid!()
    ))
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await?;
    match known {
        None => return Err(ApiError::NotFound("invalid_reset_token")),
        // Gesperrte Konten können ihr Passwort nicht selbst zurücksetzen.
        Some(true) => return Err(ApiError::Forbidden("account_disabled")),
        Some(false) => {}
    }
    let password_hash = auth::hash_password(body.new_password).await?;

    let mut tx = state.db.begin().await?;
    // Verbrauchen und Passwort setzen in einem Zug – ein Link wirkt höchstens einmal.
    let user_id: Option<Uuid> = sqlx::query_scalar(concat!(
        "UPDATE password_reset_tokens t SET used_at = now() WHERE t.token_hash = $1 AND ",
        valid!(),
        " RETURNING t.user_id"
    ))
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let user_id = user_id.ok_or(ApiError::NotFound("invalid_reset_token"))?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user_id)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;
    // Weitere offene Links desselben Kontos sind damit hinfällig, ebenso halbe 2FA-Anmeldungen.
    // Die 2FA selbst bleibt aktiv – die nächste Anmeldung fragt weiter den zweiten Faktor ab.
    for query in [
        "DELETE FROM password_reset_tokens WHERE user_id = $1 AND used_at IS NULL",
        "DELETE FROM mfa_challenges WHERE user_id = $1",
    ] {
        sqlx::query(query).bind(user_id).execute(&mut *tx).await?;
    }
    let sessions: Vec<Uuid> = sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING id")
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;

    for id in sessions {
        state.events.control(Scope::Session(id), Control::SessionRevoked);
    }
    tracing::info!(%user_id, "Passwort über Reset-Link geändert");
    Ok(StatusCode::NO_CONTENT)
}
