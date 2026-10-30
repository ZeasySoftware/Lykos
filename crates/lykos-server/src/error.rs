use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Einheitlicher Fehlertyp der HTTP-API. Antwortet immer mit `{ "error": "<code>" }`.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("ungültige Anfrage: {0}")]
    BadRequest(&'static str),
    #[error("nicht angemeldet: {0}")]
    Unauthorized(&'static str),
    #[error("keine Berechtigung: {0}")]
    Forbidden(&'static str),
    #[error("nicht gefunden: {0}")]
    NotFound(&'static str),
    #[error("Konflikt: {0}")]
    Conflict(&'static str),
    /// Zu viele Versuche; `retry_after` in Sekunden (→ `Retry-After`-Header).
    #[error("zu viele Versuche, erneut in {retry_after} s")]
    RateLimited { retry_after: u64 },
    #[error("zu groß: {0}")]
    PayloadTooLarge(&'static str),
    /// Vom AutoMod der Community blockiert: `403 { error: "automod_blocked", rule, rule_name, message }`.
    #[error("vom AutoMod blockiert: {rule}")]
    AutomodBlocked { rule: &'static str, rule_name: &'static str, message: String },
    #[error("nicht verfügbar: {0}")]
    ServiceUnavailable(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    pub fn forbidden() -> Self {
        Self::Forbidden("forbidden")
    }

    pub fn not_found() -> Self {
        Self::NotFound("not_found")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let ApiError::RateLimited { retry_after } = self {
            let mut res =
                (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "rate_limited" }))).into_response();
            res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
            return res;
        }
        if let ApiError::AutomodBlocked { rule, rule_name, message } = self {
            let body = json!({ "error": "automod_blocked", "rule": rule, "rule_name": rule_name, "message": message });
            return (StatusCode::FORBIDDEN, Json(body)).into_response();
        }
        let (status, code) = match &self {
            ApiError::BadRequest(c) => (StatusCode::BAD_REQUEST, *c),
            ApiError::Unauthorized(c) => (StatusCode::UNAUTHORIZED, *c),
            ApiError::Forbidden(c) => (StatusCode::FORBIDDEN, *c),
            ApiError::NotFound(c) => (StatusCode::NOT_FOUND, *c),
            ApiError::Conflict(c) => (StatusCode::CONFLICT, *c),
            ApiError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            ApiError::PayloadTooLarge(c) => (StatusCode::PAYLOAD_TOO_LARGE, *c),
            ApiError::ServiceUnavailable(c) => (StatusCode::SERVICE_UNAVAILABLE, *c),
            ApiError::AutomodBlocked { .. } => (StatusCode::FORBIDDEN, "automod_blocked"),
            ApiError::Database(_) | ApiError::Internal(_) => {
                tracing::error!(error = %self, "interner Fehler");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        (status, Json(json!({ "error": code }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Prüft, ob ein DB-Fehler eine Unique-Verletzung ist (z. B. doppelter Benutzername).
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(e) if e.is_unique_violation())
}
