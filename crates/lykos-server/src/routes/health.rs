use axum::{Json, extract::State};
use serde::Serialize;

use crate::{error::ApiResult, state::AppState};

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    version: &'static str,
    database: &'static str,
}

pub async fn health(State(state): State<AppState>) -> ApiResult<Json<Health>> {
    let database = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "ok",
        Err(e) => {
            tracing::warn!(error = %e, "Datenbank nicht erreichbar");
            "unreachable"
        }
    };

    Ok(Json(Health {
        status: if database == "ok" { "ok" } else { "degraded" },
        version: env!("CARGO_PKG_VERSION"),
        database,
    }))
}
