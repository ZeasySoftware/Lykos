//! Speicherbelegung für Anhänge (Kontingente, siehe `quota.rs`).

use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::ApiResult,
    permissions::{self, Permissions},
    quota::{self, Usage},
    state::AppState,
};

/// `GET /auth/me/usage` – eigene Belegung und Kontingent.
pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Usage>> {
    Ok(Json(quota::user_usage(&state, user.user_id).await?))
}

/// `GET /communities/{id}/usage` – Belegung der Community (`MANAGE_COMMUNITY`).
pub async fn community(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Usage>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;
    Ok(Json(quota::community_usage(&state, community_id).await?))
}
