use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events,
    models::{self, Role, double_option},
    permissions::{self, MemberContext, Permissions},
    routes::channels::permissions_changed,
    state::AppState,
};

macro_rules! role_columns {
    () => {
        "id, community_id, name, permissions, position, color, is_default"
    };
}

fn validate_role_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if (1..=64).contains(&name.chars().count()) {
        Ok(name.to_owned())
    } else {
        Err(ApiError::BadRequest("invalid_name"))
    }
}

fn validate_color(color: Option<i32>) -> ApiResult<Option<i32>> {
    match color {
        Some(c) if !(0..=0xFF_FFFF).contains(&c) => Err(ApiError::BadRequest("invalid_request")),
        c => Ok(c),
    }
}

/// Position der höchsten eigenen Rolle (Standardrolle ausgenommen); `None` = nur Standardrolle.
/// Kleinere Position = höher in der Hierarchie, die Standardrolle steht immer ganz unten.
pub async fn top_position(db: &PgPool, ctx: &MemberContext) -> ApiResult<Option<i32>> {
    let ids: Vec<Uuid> =
        ctx.roles.iter().map(|(id, _)| *id).filter(|id| *id != ctx.default_role_id).collect();
    if ids.is_empty() {
        return Ok(None);
    }
    Ok(sqlx::query_scalar("SELECT min(position) FROM roles WHERE id = ANY($1) AND NOT is_default")
        .bind(&ids)
        .fetch_one(db)
        .await?)
}

/// Darf ein Mitglied mit der höchsten Rolle an Position `top` eine Rolle an `position` verwalten
/// (bearbeiten, löschen, verschieben, zuweisen)? Der Besitzer immer; sonst nur Rollen echt unterhalb
/// der eigenen höchsten Rolle. Die Standardrolle liegt unter allen anderen.
pub fn outranks(is_owner: bool, top: Option<i32>, role_is_default: bool, position: i32) -> bool {
    is_owner || role_is_default || top.is_some_and(|top| position > top)
}

/// Prüft die Rollenhierarchie; `Forbidden`, wenn die Rolle nicht unterhalb der eigenen höchsten liegt.
pub async fn require_outranks(db: &PgPool, ctx: &MemberContext, role: &Role) -> ApiResult<()> {
    if ctx.is_owner || role.is_default {
        return Ok(());
    }
    let top = top_position(db, ctx).await?;
    if outranks(false, top, false, role.position) { Ok(()) } else { Err(ApiError::forbidden()) }
}

/// Hierarchie gegenüber einem anderen Mitglied (z. B. fremden Nickname ändern): Der Besitzer darf
/// immer; sonst muss die höchste Rolle des Ziels echt unter der eigenen höchsten Rolle stehen
/// (ein Ziel nur mit Standardrolle steht unter jeder anderen Rolle).
pub async fn require_outranks_member(db: &PgPool, ctx: &MemberContext, target_id: Uuid) -> ApiResult<()> {
    if ctx.is_owner {
        return Ok(());
    }
    let Some(own) = top_position(db, ctx).await? else {
        return Err(ApiError::forbidden());
    };
    let target: Option<i32> = sqlx::query_scalar(
        "SELECT min(r.position) FROM member_roles mr JOIN roles r ON r.id = mr.role_id
         WHERE mr.community_id = $1 AND mr.user_id = $2 AND NOT r.is_default",
    )
    .bind(ctx.community_id)
    .bind(target_id)
    .fetch_one(db)
    .await?;
    if target.is_none_or(|t| t > own) { Ok(()) } else { Err(ApiError::forbidden()) }
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    permissions: Option<i64>,
    color: Option<i32>,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Role>)> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_ROLES)?;

    let name = validate_role_name(&body.name)?;
    let perms = Permissions(body.permissions.unwrap_or(0)).sanitized();
    permissions::require(ctx.base(), perms)?;
    let color = validate_color(body.color)?;

    let mut tx = state.db.begin().await?;
    let role: Role = sqlx::query_as(concat!(
        "INSERT INTO roles (id, community_id, name, permissions, color, position)
         VALUES ($1, $2, $3, $4, $5,
                 (SELECT COALESCE(max(position) + 1, 1) FROM roles WHERE community_id = $2))
         RETURNING ",
        role_columns!()
    ))
    .bind(Uuid::now_v7())
    .bind(community_id)
    .bind(&name)
    .bind(perms.0)
    .bind(color)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "role.create",
        Some(role.id),
        json!({ "name": name }),
    )
    .await?;
    tx.commit().await?;

    state.events.publish(events::community(community_id), "ROLE_CREATE", &role);
    Ok((StatusCode::CREATED, Json(role)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    permissions: Option<i64>,
    #[serde(default, deserialize_with = "double_option")]
    color: Option<Option<i32>>,
    position: Option<i32>,
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(role_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Role>> {
    let role = models::role_by_id(&state.db, role_id).await?.ok_or_else(ApiError::not_found)?;
    let ctx = permissions::member_context(&state.db, user.user_id, role.community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_ROLES)?;
    // Wer eine Rolle bearbeitet, muss deren bisherige und neue Rechte selbst besitzen.
    permissions::require(ctx.base(), role.permissions)?;
    require_outranks(&state.db, &ctx, &role).await?;
    if let Some(position) = body.position {
        // Die Standardrolle hat keinen Platz in der Reihenfolge; andere nur unter die eigene höchste Rolle.
        if role.is_default || position < 1 {
            return Err(ApiError::BadRequest("invalid_request"));
        }
        let top = if ctx.is_owner { None } else { top_position(&state.db, &ctx).await? };
        if !outranks(ctx.is_owner, top, false, position) {
            return Err(ApiError::forbidden());
        }
    }

    let name = body.name.as_deref().map(validate_role_name).transpose()?;
    let perms = body.permissions.map(|p| Permissions(p).sanitized());
    if let Some(p) = perms {
        permissions::require(ctx.base(), p)?;
    }
    let color = body.color.map(validate_color).transpose()?;

    let mut tx = state.db.begin().await?;
    let updated: Role = sqlx::query_as(concat!(
        "UPDATE roles SET
            name = COALESCE($2, name),
            permissions = COALESCE($3, permissions),
            color = CASE WHEN $4 THEN $5 ELSE color END,
            position = COALESCE($6, position)
         WHERE id = $1 RETURNING ",
        role_columns!()
    ))
    .bind(role_id)
    .bind(&name)
    .bind(perms.map(|p| p.0))
    .bind(color.is_some())
    .bind(color.flatten())
    .bind(body.position)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        role.community_id,
        user.user_id,
        "role.update",
        Some(role_id),
        json!({ "name": name, "permissions": perms }),
    )
    .await?;
    tx.commit().await?;

    state.events.publish(events::community(role.community_id), "ROLE_UPDATE", &updated);
    if perms.is_some_and(|p| p != role.permissions) {
        permissions_changed(&state, role.community_id);
    }
    Ok(Json(updated))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(role_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let role = models::role_by_id(&state.db, role_id).await?.ok_or_else(ApiError::not_found)?;
    let ctx = permissions::member_context(&state.db, user.user_id, role.community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_ROLES)?;
    permissions::require(ctx.base(), role.permissions)?;
    if role.is_default {
        return Err(ApiError::forbidden());
    }
    require_outranks(&state.db, &ctx, &role).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM roles WHERE id = $1").bind(role_id).execute(&mut *tx).await?;
    audit::record(
        &mut *tx,
        role.community_id,
        user.user_id,
        "role.delete",
        Some(role_id),
        json!({ "name": role.name }),
    )
    .await?;
    tx.commit().await?;

    state.events.publish(
        events::community(role.community_id),
        "ROLE_DELETE",
        json!({ "id": role_id, "community_id": role.community_id }),
    );
    permissions_changed(&state, role.community_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReorderBody {
    /// Alle Rollen außer der Standardrolle, von der höchsten zur niedrigsten.
    role_ids: Vec<Uuid>,
}

/// `PATCH /communities/{id}/roles/order` – setzt die Reihenfolge aller Rollen auf einmal
/// (Positionen 1..n). Ohne Besitz bleiben die eigene höchste Rolle und alles darüber an ihrem Platz.
pub async fn reorder(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<ReorderBody>,
) -> ApiResult<Json<Vec<Role>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_ROLES)?;
    let top = if ctx.is_owner { None } else { top_position(&state.db, &ctx).await? };

    let mut tx = state.db.begin().await?;
    // `FOR UPDATE` serialisiert parallele Umsortierungen derselben Community.
    let current: Vec<Role> = sqlx::query_as(concat!(
        "SELECT ",
        role_columns!(),
        " FROM roles WHERE community_id = $1 AND NOT is_default ORDER BY position, id FOR UPDATE"
    ))
    .bind(community_id)
    .fetch_all(&mut *tx)
    .await?;

    let wanted: std::collections::HashSet<Uuid> = body.role_ids.iter().copied().collect();
    if wanted.len() != body.role_ids.len()
        || wanted.len() != current.len()
        || !current.iter().all(|r| wanted.contains(&r.id))
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }

    if !ctx.is_owner {
        let fixed = current.iter().take_while(|r| !outranks(false, top, false, r.position)).count();
        if current[..fixed].iter().zip(&body.role_ids).any(|(r, id)| r.id != *id) {
            return Err(ApiError::forbidden());
        }
    }

    let mut changed = Vec::new();
    for (index, role_id) in body.role_ids.iter().enumerate() {
        let position = i32::try_from(index + 1).map_err(|_| ApiError::BadRequest("invalid_request"))?;
        if current.iter().any(|r| r.id == *role_id && r.position == position) {
            continue;
        }
        let role: Role = sqlx::query_as(concat!(
            "UPDATE roles SET position = $2 WHERE id = $1 RETURNING ",
            role_columns!()
        ))
        .bind(role_id)
        .bind(position)
        .fetch_one(&mut *tx)
        .await?;
        changed.push(role);
    }
    if !changed.is_empty() {
        audit::record(
            &mut *tx,
            community_id,
            user.user_id,
            "role.reorder",
            None,
            json!({ "role_ids": body.role_ids }),
        )
        .await?;
    }
    tx.commit().await?;

    for role in &changed {
        state.events.publish(events::community(community_id), "ROLE_UPDATE", role);
    }
    Ok(Json(models::roles_of(&state.db, community_id).await?))
}

#[cfg(test)]
mod tests {
    use super::outranks;

    #[test]
    fn hierarchy() {
        // Besitzer und Standardrolle: immer.
        assert!(outranks(true, None, false, 1));
        assert!(outranks(false, None, true, 0));
        // Nur die Standardrolle: keine andere Rolle verwaltbar.
        assert!(!outranks(false, None, false, 5));
        // Höchste eigene Rolle an Position 2: nur Rollen darunter (3, 4, …).
        assert!(!outranks(false, Some(2), false, 1));
        assert!(!outranks(false, Some(2), false, 2));
        assert!(outranks(false, Some(2), false, 3));
    }
}
