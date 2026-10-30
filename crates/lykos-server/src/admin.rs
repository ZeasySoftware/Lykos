//! Instanz-Admins: zentrale Rechteprüfung (`InstanceAdmin`-Extractor) und Laufzeitdaten fürs Dashboard.

use std::{sync::OnceLock, time::Instant};

use axum::{extract::FromRequestParts, http::request::Parts};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Ist der Nutzer (nicht gesperrter) Instanz-Admin?
pub async fn is_instance_admin(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND is_instance_admin AND disabled_at IS NULL)",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?)
}

/// Angemeldeter Instanz-Admin. Als Handler-Argument erzwingt er Anmeldung (`401`) und Admin-Recht
/// (`403 forbidden`) – alle `/admin/*`-Routen prüfen ausschließlich hierüber.
#[derive(Debug, Clone, Copy)]
pub struct InstanceAdmin(pub AuthUser);

impl InstanceAdmin {
    pub fn user_id(&self) -> Uuid {
        self.0.user_id
    }
}

impl FromRequestParts<AppState> for InstanceAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !is_instance_admin(&state.db, user.user_id).await? {
            return Err(ApiError::forbidden());
        }
        Ok(Self(user))
    }
}

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Startzeitpunkt merken (beim Aufbau des Anwendungszustands; mehrfacher Aufruf ist harmlos).
pub fn mark_started() {
    STARTED.get_or_init(Instant::now);
}

/// Laufzeit des Prozesses in Sekunden.
pub fn uptime_seconds() -> u64 {
    STARTED.get_or_init(Instant::now).elapsed().as_secs()
}
