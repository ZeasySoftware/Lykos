//! Communities-Übersicht für Instanz-Admins: auflisten, löschen, Besitz übertragen – ohne Mitgliedschaft
//! und ohne Einsicht in Kanäle oder Nachrichten (Moderationsfall).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ListQuery, Page};
use crate::{
    admin::InstanceAdmin,
    audit,
    error::{ApiError, ApiResult},
    models::{Community, UserPublic},
    routes::communities,
    state::AppState,
};

#[derive(Serialize)]
pub struct AdminCommunity {
    id: Uuid,
    name: String,
    owner: UserPublic,
    member_count: i64,
    channel_count: i64,
    /// Speicherbelegung durch Anhänge in Bytes.
    storage_bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    name: String,
    created_at: OffsetDateTime,
    owner_id: Uuid,
    owner_username: String,
    owner_display_name: Option<String>,
    owner_avatar_id: Option<Uuid>,
    member_count: i64,
    channel_count: i64,
    storage_bytes: i64,
    total: i64,
}

/// `GET /admin/communities?q=&page=&per_page=`
pub async fn list(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Page<AdminCommunity>>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.name, c.created_at, u.id AS owner_id, u.username AS owner_username,
                u.display_name AS owner_display_name, u.avatar_id AS owner_avatar_id,
                (SELECT count(*) FROM members m WHERE m.community_id = c.id) AS member_count,
                (SELECT count(*) FROM channels ch WHERE ch.community_id = c.id) AS channel_count,
                (SELECT COALESCE(sum(a.size_bytes), 0)::BIGINT FROM attachments a WHERE a.community_id = c.id)
                    AS storage_bytes,
                count(*) OVER () AS total
         FROM communities c JOIN users u ON u.id = c.owner_id
         WHERE ($1::text IS NULL OR c.name ILIKE $1)
         ORDER BY c.created_at DESC, c.id DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(q.pattern())
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&state.db)
    .await?;
    let total = rows.first().map_or(0, |r| r.total);
    Ok(Json(Page {
        total,
        items: rows
            .into_iter()
            .map(|r| AdminCommunity {
                id: r.id,
                name: r.name,
                owner: UserPublic::new(r.owner_id, r.owner_username, r.owner_display_name, r.owner_avatar_id),
                member_count: r.member_count,
                channel_count: r.channel_count,
                storage_bytes: r.storage_bytes,
                created_at: r.created_at,
            })
            .collect(),
    }))
}

async fn community_name(state: &AppState, community_id: Uuid) -> ApiResult<(String, Uuid)> {
    sqlx::query_as("SELECT name, owner_id FROM communities WHERE id = $1")
        .bind(community_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}

/// `GET /admin/communities/{id}/members` – Mitglieder (nur Namen) zur Auswahl eines neuen Besitzers.
pub async fn members(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<UserPublic>>> {
    community_name(&state, community_id).await?;
    let rows: Vec<(Uuid, String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, u.avatar_id
         FROM members m JOIN users u ON u.id = m.user_id
         WHERE m.community_id = $1 ORDER BY lower(u.username)",
    )
    .bind(community_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(|(id, u, d, a)| UserPublic::new(id, u, d, a)).collect()))
}

/// `DELETE /admin/communities/{id}` – Community samt Inhalt löschen.
pub async fn delete(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(community_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (name, owner_id) = community_name(&state, community_id).await?;
    audit::record_instance(
        &state.db,
        admin.user_id(),
        "community.delete",
        Some(community_id),
        json!({ "name": name, "owner_id": owner_id }),
    )
    .await?;
    communities::delete_community(&state, community_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TransferBody {
    user_id: Uuid,
}

/// `POST /admin/communities/{id}/transfer` – Besitz an ein Mitglied übertragen.
pub async fn transfer(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(community_id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> ApiResult<Json<Community>> {
    let (name, owner_id) = community_name(&state, community_id).await?;
    if body.user_id == owner_id {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let community =
        communities::transfer_ownership(&state, community_id, owner_id, body.user_id, admin.user_id())
            .await?;
    audit::record_instance(
        &state.db,
        admin.user_id(),
        "community.transfer",
        Some(community_id),
        json!({ "name": name, "from": owner_id, "to": body.user_id }),
    )
    .await?;
    Ok(Json(community))
}
