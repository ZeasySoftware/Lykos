//! Instanz-Einladungen: Registrierungs-Einladungen ohne Community-Bindung (optional mit automatischem
//! Beitritt zu einer Community). Eingelöst werden sie in `routes::invites::redeem_for_registration`.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::base_url;
use crate::{
    admin::InstanceAdmin,
    audit, auth,
    error::{ApiError, ApiResult},
    models::UserPublic,
    state::AppState,
};

/// Zufallsbytes eines Instanz-Einladungscodes → 12 Zeichen (Community-Einladungen haben 8).
const CODE_BYTES: usize = 9;
const NOTE_MAX_CHARS: usize = 200;

#[derive(Serialize)]
pub struct CommunityRef {
    id: Uuid,
    name: String,
}

#[derive(Serialize)]
pub struct Redemption {
    user: UserPublic,
    #[serde(with = "time::serde::rfc3339")]
    used_at: OffsetDateTime,
}

#[derive(Serialize)]
pub struct InstanceInvite {
    code: String,
    /// Einladungslink `{public_url}/invite/{code}` (gleiche Route wie Community-Einladungen).
    url: String,
    note: Option<String>,
    /// Neue Nutzer treten dieser Community automatisch bei.
    community: Option<CommunityRef>,
    max_uses: Option<i32>,
    uses: i32,
    #[serde(with = "time::serde::rfc3339::option")]
    expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    revoked_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    created_by: Option<UserPublic>,
    /// `active`, `expired`, `used_up` oder `revoked`.
    status: &'static str,
    /// Wer sich damit registriert hat (gelöschte Konten fehlen).
    used_by: Vec<Redemption>,
}

#[derive(FromRow)]
struct Row {
    code: String,
    note: Option<String>,
    community_id: Option<Uuid>,
    community_name: Option<String>,
    max_uses: Option<i32>,
    uses: i32,
    expires_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    creator_id: Option<Uuid>,
    creator_username: Option<String>,
    creator_display_name: Option<String>,
    creator_avatar_id: Option<Uuid>,
}

macro_rules! select_invites {
    () => {
        "SELECT i.code, i.note, i.community_id, c.name AS community_name, i.max_uses, i.uses, i.expires_at,
                i.revoked_at, i.created_at, u.id AS creator_id, u.username AS creator_username,
                u.display_name AS creator_display_name, u.avatar_id AS creator_avatar_id
         FROM instance_invites i
         LEFT JOIN communities c ON c.id = i.community_id
         LEFT JOIN users u ON u.id = i.created_by"
    };
}

fn status(row: &Row, now: OffsetDateTime) -> &'static str {
    if row.revoked_at.is_some() {
        "revoked"
    } else if row.expires_at.is_some_and(|e| e <= now) {
        "expired"
    } else if row.max_uses.is_some_and(|m| row.uses >= m) {
        "used_up"
    } else {
        "active"
    }
}

#[derive(FromRow)]
struct UseRow {
    code: String,
    id: Uuid,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    used_at: OffsetDateTime,
}

async fn load(state: &AppState, rows: Vec<Row>) -> ApiResult<Vec<InstanceInvite>> {
    let codes: Vec<&str> = rows.iter().map(|r| r.code.as_str()).collect();
    let uses: Vec<UseRow> = sqlx::query_as(
        "SELECT iu.code, u.id, u.username, u.display_name, u.avatar_id, iu.used_at
         FROM instance_invite_uses iu JOIN users u ON u.id = iu.user_id
         WHERE iu.code = ANY($1) ORDER BY iu.used_at",
    )
    .bind(&codes)
    .fetch_all(&state.db)
    .await?;
    let mut by_code: HashMap<String, Vec<Redemption>> = HashMap::new();
    for u in uses {
        by_code.entry(u.code).or_default().push(Redemption {
            user: UserPublic::new(u.id, u.username, u.display_name, u.avatar_id),
            used_at: u.used_at,
        });
    }

    let now = OffsetDateTime::now_utc();
    let base = base_url(state);
    Ok(rows
        .into_iter()
        .map(|r| InstanceInvite {
            status: status(&r, now),
            url: format!("{base}/invite/{}", r.code),
            used_by: by_code.remove(&r.code).unwrap_or_default(),
            community: r.community_id.zip(r.community_name).map(|(id, name)| CommunityRef { id, name }),
            created_by: match (r.creator_id, r.creator_username) {
                (Some(id), Some(username)) => {
                    Some(UserPublic::new(id, username, r.creator_display_name, r.creator_avatar_id))
                }
                _ => None,
            },
            code: r.code,
            note: r.note,
            max_uses: r.max_uses,
            uses: r.uses,
            expires_at: r.expires_at,
            revoked_at: r.revoked_at,
            created_at: r.created_at,
        })
        .collect())
}

/// `GET /admin/invites` – alle Instanz-Einladungen (auch abgelaufene/widerrufene), neueste zuerst.
pub async fn list(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
) -> ApiResult<Json<Vec<InstanceInvite>>> {
    let rows: Vec<Row> = sqlx::query_as(concat!(select_invites!(), " ORDER BY i.created_at DESC LIMIT 500"))
        .fetch_all(&state.db)
        .await?;
    Ok(Json(load(&state, rows).await?))
}

#[derive(Deserialize)]
pub struct CreateBody {
    max_uses: Option<i32>,
    expires_in_hours: Option<i64>,
    note: Option<String>,
    /// Neue Nutzer treten dieser Community automatisch bei.
    community_id: Option<Uuid>,
}

/// `POST /admin/invites` – Instanz-Einladung anlegen.
pub async fn create(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<InstanceInvite>)> {
    if body.max_uses.is_some_and(|n| !(1..=10_000).contains(&n))
        || body.expires_in_hours.is_some_and(|h| !(1..=24 * 365).contains(&h))
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let note = auth::normalize_text(body.note, NOTE_MAX_CHARS, "invalid_note")?;
    if let Some(community_id) = body.community_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM communities WHERE id = $1)")
            .bind(community_id)
            .fetch_one(&state.db)
            .await?;
        if !exists {
            return Err(ApiError::NotFound("community_not_found"));
        }
    }
    let expires_at = body.expires_in_hours.map(|h| OffsetDateTime::now_utc() + Duration::hours(h));
    let code = auth::random_token(CODE_BYTES);

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO instance_invites (code, created_by, note, community_id, max_uses, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&code)
    .bind(admin.user_id())
    .bind(&note)
    .bind(body.community_id)
    .bind(body.max_uses)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    audit::record_instance(
        &mut *tx,
        admin.user_id(),
        "instance_invite.create",
        None,
        json!({
            "code": code,
            "note": note,
            "community_id": body.community_id,
            "max_uses": body.max_uses,
            "expires_in_hours": body.expires_in_hours,
        }),
    )
    .await?;
    tx.commit().await?;

    let rows: Vec<Row> = sqlx::query_as(concat!(select_invites!(), " WHERE i.code = $1"))
        .bind(&code)
        .fetch_all(&state.db)
        .await?;
    let invite = load(&state, rows).await?.pop().ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(invite)))
}

/// `DELETE /admin/invites/{code}` – widerrufen. Die Einladung bleibt zur Nachvollziehbarkeit in der Liste.
pub async fn revoke(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let found =
        sqlx::query("UPDATE instance_invites SET revoked_at = COALESCE(revoked_at, now()) WHERE code = $1")
            .bind(&code)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if found == 0 {
        return Err(ApiError::NotFound("invalid_invite"));
    }
    audit::record_instance(
        &mut *tx,
        admin.user_id(),
        "instance_invite.revoke",
        None,
        json!({ "code": code }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
