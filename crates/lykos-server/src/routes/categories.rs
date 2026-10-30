//! Kanal-Kategorien: anlegen, umbenennen, löschen und sortieren (alles mit `MANAGE_CHANNELS`).
//!
//! Kategorien haben keine eigenen Rechte. Ein Mitglied sieht eine Kategorie, wenn es mindestens einen ihrer
//! Kanäle sehen darf oder `MANAGE_CHANNELS` hat (siehe `is_visible`); dieselbe Regel nutzt das Gateway für
//! die Events `CATEGORY_CREATE`/`CATEGORY_UPDATE` (kanalgebundener Scope mit der Kategorie-ID).

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events,
    models::{self, Category},
    permissions::{self, Permissions},
    state::AppState,
};

/// Höchstzahl der Kategorien pro Community (wie bei Discord liegt die Grenze weit über üblichem Bedarf).
pub const MAX_CATEGORIES: i64 = 100;
/// Höchstlänge eines Kategorienamens (Zeichen) – wie bei Kanälen.
pub const NAME_MAX_CHARS: usize = 64;

/// Kategorienamen: getrimmt, ohne Steuerzeichen, 1–64 Zeichen; Groß-/Kleinschreibung bleibt.
pub fn normalize_name(name: &str) -> ApiResult<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if (1..=NAME_MAX_CHARS).contains(&name.chars().count()) {
        Ok(name.to_owned())
    } else {
        Err(ApiError::BadRequest("invalid_name"))
    }
}

/// Sieht ein Mitglied die Kategorie? Ja mit `MANAGE_CHANNELS` (Community-Ebene) oder wenn einer der für
/// es sichtbaren Kanäle (`visible_channel_categories` = deren `category_id`) in ihr liegt.
pub fn is_visible(
    category_id: Uuid,
    can_manage: bool,
    mut visible_channel_categories: impl Iterator<Item = Option<Uuid>>,
) -> bool {
    can_manage || visible_channel_categories.any(|c| c == Some(category_id))
}

/// Gehört die Kategorie zur Community? Sonst `invalid_category` (für Kanal-Erstellen/-PATCH).
pub async fn require_in_community(
    db: impl sqlx::PgExecutor<'_>,
    category_id: Uuid,
    community_id: Uuid,
) -> ApiResult<()> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channel_categories WHERE id = $1 AND community_id = $2)",
    )
    .bind(category_id)
    .bind(community_id)
    .fetch_one(db)
    .await?;
    if ok { Ok(()) } else { Err(ApiError::BadRequest("invalid_category")) }
}

/// Lädt die Kategorie und prüft `MANAGE_CHANNELS` in ihrer Community (`NotFound` für Nicht-Mitglieder).
async fn managed_category(state: &AppState, user_id: Uuid, category_id: Uuid) -> ApiResult<Category> {
    let category = models::category_by_id(&state.db, category_id).await?.ok_or_else(ApiError::not_found)?;
    let ctx = permissions::member_context(&state.db, user_id, category.community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_CHANNELS)?;
    Ok(category)
}

/// Scope der Kategorie-Events: nur Mitglieder, die die Kategorie sehen (siehe Modul-Doku).
fn scope(category: &Category) -> events::Scope {
    events::channel(category.community_id, category.id)
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
}

/// `POST /communities/{id}/categories` – neue Kategorie ans Ende.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Category>)> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_CHANNELS)?;
    let name = normalize_name(&body.name)?;

    let mut tx = state.db.begin().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_categories WHERE community_id = $1")
        .bind(community_id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_CATEGORIES {
        return Err(ApiError::Conflict("too_many_categories"));
    }
    let category: Category = sqlx::query_as(concat!(
        "INSERT INTO channel_categories (id, community_id, name, position)
         VALUES ($1, $2, $3, (SELECT COALESCE(max(position) + 1, 0) FROM channel_categories WHERE community_id = $2))
         RETURNING ",
        crate::category_columns!()
    ))
    .bind(Uuid::now_v7())
    .bind(community_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "category.create",
        Some(category.id),
        json!({ "name": name }),
    )
    .await?;
    tx.commit().await?;

    // Neue ID: Verbindungen müssen ihren Sichtbarkeits-Cache neu berechnen, um sie zu kennen.
    state.events.publish_permissions(scope(&category), "CATEGORY_CREATE", &category);
    Ok((StatusCode::CREATED, Json(category)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    position: Option<i32>,
}

/// `PATCH /categories/{id}` – umbenennen und/oder Position setzen.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(category_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Category>> {
    let current = managed_category(&state, user.user_id, category_id).await?;
    let name = body.name.as_deref().map(normalize_name).transpose()?;
    if body.position.is_some_and(|p| p < 0) {
        return Err(ApiError::BadRequest("invalid_request"));
    }

    let mut tx = state.db.begin().await?;
    let category: Category = sqlx::query_as(concat!(
        "UPDATE channel_categories SET name = COALESCE($2, name), position = COALESCE($3, position)
         WHERE id = $1 RETURNING ",
        crate::category_columns!()
    ))
    .bind(category_id)
    .bind(&name)
    .bind(body.position)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        current.community_id,
        user.user_id,
        "category.update",
        Some(category_id),
        json!({ "name": name, "position": body.position }),
    )
    .await?;
    tx.commit().await?;

    state.events.publish(scope(&category), "CATEGORY_UPDATE", &category);
    Ok(Json(category))
}

/// `DELETE /categories/{id}` – Kategorie löschen; ihre Kanäle bleiben und stehen danach ohne Kategorie.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(category_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let category = managed_category(&state, user.user_id, category_id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM channel_categories WHERE id = $1").bind(category_id).execute(&mut *tx).await?;
    audit::record(
        &mut *tx,
        category.community_id,
        user.user_id,
        "category.delete",
        Some(category_id),
        json!({ "name": category.name }),
    )
    .await?;
    tx.commit().await?;

    // An die ganze Community (nur die ID): wer die Kategorie sah, ist nach dem Löschen nicht mehr prüfbar.
    state.events.publish_permissions(
        events::community(category.community_id),
        "CATEGORY_DELETE",
        json!({ "id": category_id, "community_id": category.community_id }),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReorderBody {
    category_ids: Vec<Uuid>,
}

/// `PATCH /communities/{id}/categories/order` – alle Kategorien in neuer Reihenfolge (oben zuerst).
pub async fn reorder(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<ReorderBody>,
) -> ApiResult<Json<Vec<Category>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_CHANNELS)?;

    let mut tx = state.db.begin().await?;
    // `FOR UPDATE` serialisiert parallele Umsortierungen derselben Community.
    let current: Vec<Category> = sqlx::query_as(concat!(
        "SELECT ",
        crate::category_columns!(),
        " FROM channel_categories WHERE community_id = $1 ORDER BY position, id FOR UPDATE"
    ))
    .bind(community_id)
    .fetch_all(&mut *tx)
    .await?;
    let wanted: HashSet<Uuid> = body.category_ids.iter().copied().collect();
    if wanted.len() != body.category_ids.len()
        || wanted.len() != current.len()
        || !current.iter().all(|c| wanted.contains(&c.id))
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }

    let mut changed = Vec::new();
    for (index, id) in body.category_ids.iter().enumerate() {
        let position = i32::try_from(index).map_err(|_| ApiError::BadRequest("invalid_request"))?;
        if current.iter().any(|c| c.id == *id && c.position == position) {
            continue;
        }
        let category: Category = sqlx::query_as(concat!(
            "UPDATE channel_categories SET position = $2 WHERE id = $1 RETURNING ",
            crate::category_columns!()
        ))
        .bind(id)
        .bind(position)
        .fetch_one(&mut *tx)
        .await?;
        changed.push(category);
    }
    if !changed.is_empty() {
        audit::record(
            &mut *tx,
            community_id,
            user.user_id,
            "category.reorder",
            None,
            json!({ "category_ids": body.category_ids }),
        )
        .await?;
    }
    tx.commit().await?;

    for category in &changed {
        state.events.publish(scope(category), "CATEGORY_UPDATE", category);
    }
    Ok(Json(models::categories_of(&state.db, community_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_limited() {
        assert_eq!(normalize_name("  Spiele & Co  ").unwrap(), "Spiele & Co");
        assert_eq!(normalize_name("Mit\u{7}Glocke").unwrap(), "MitGlocke");
        assert!(normalize_name("   ").is_err());
        assert!(normalize_name(&"x".repeat(65)).is_err());
        assert!(normalize_name(&"x".repeat(64)).is_ok());
    }

    #[test]
    fn visibility_needs_a_visible_channel_or_manage() {
        let cat = Uuid::now_v7();
        let other = Uuid::now_v7();
        assert!(!is_visible(cat, false, [None, Some(other)].into_iter()));
        assert!(is_visible(cat, false, [None, Some(cat)].into_iter()));
        assert!(is_visible(cat, true, std::iter::empty()));
    }
}
