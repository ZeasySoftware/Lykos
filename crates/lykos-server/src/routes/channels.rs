use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events,
    models::{self, Channel, double_option},
    permissions::{self, Permissions},
    routes::categories,
    state::AppState,
    voice,
};

/// Textkanäle: klein, Leerzeichen → `-`. Voice-Kanäle behalten ihren Namen. 1–64 Zeichen.
pub fn normalize_channel_name(name: &str, kind: &str) -> ApiResult<String> {
    let name = name.trim();
    let name = if kind == "text" {
        name.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
    } else {
        name.to_owned()
    };
    if (1..=64).contains(&name.chars().count()) {
        Ok(name)
    } else {
        Err(ApiError::BadRequest("invalid_name"))
    }
}

fn normalize_topic(topic: Option<String>) -> ApiResult<Option<String>> {
    let topic = topic.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    if topic.as_ref().is_some_and(|t| t.chars().count() > 1024) {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    Ok(topic)
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    kind: String,
    topic: Option<String>,
    /// Kategorie in derselben Community; fehlt/`null` = ohne Kategorie.
    category_id: Option<Uuid>,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_CHANNELS)?;

    if !matches!(body.kind.as_str(), "text" | "voice") {
        return Err(ApiError::BadRequest("invalid_kind"));
    }
    let name = normalize_channel_name(&body.name, &body.kind)?;
    let topic = normalize_topic(body.topic)?;
    if let Some(category_id) = body.category_id {
        categories::require_in_community(&state.db, category_id, community_id).await?;
    }

    let mut tx = state.db.begin().await?;
    let channel: Channel = sqlx::query_as(concat!(
        "INSERT INTO channels (id, community_id, name, kind, topic, position, category_id)
         VALUES ($1, $2, $3, $4, $5,
                 (SELECT COALESCE(max(position) + 1, 0) FROM channels WHERE community_id = $2 AND kind = $4),
                 $6)
         RETURNING ",
        crate::channel_columns!()
    ))
    .bind(Uuid::now_v7())
    .bind(community_id)
    .bind(&name)
    .bind(&body.kind)
    .bind(&topic)
    .bind(body.category_id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "channel.create",
        Some(channel.id),
        json!({ "name": name, "kind": body.kind, "category_id": body.category_id }),
    )
    .await?;
    tx.commit().await?;

    state.events.publish_permissions(events::channel(community_id, channel.id), "CHANNEL_CREATE", &channel);
    Ok((StatusCode::CREATED, Json(channel)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    topic: Option<Option<String>>,
    position: Option<i32>,
    /// Ton-Einstellungen, nur bei Sprachkanälen.
    audio_bitrate: Option<i32>,
    audio_stereo: Option<bool>,
    audio_dtx: Option<bool>,
    audio_red: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    user_limit: Option<Option<i32>>,
    /// Kategorie wechseln (`null` = ohne Kategorie); muss zur Community des Kanals gehören.
    #[serde(default, deserialize_with = "double_option")]
    category_id: Option<Option<Uuid>>,
}

/// Grenzen der Kanal-Bitrate in kbit/s.
pub const AUDIO_BITRATE_RANGE: std::ops::RangeInclusive<i32> = 32..=256;
/// Grenzen des Nutzerlimits.
pub const USER_LIMIT_RANGE: std::ops::RangeInclusive<i32> = 1..=99;

impl UpdateBody {
    fn touches_audio(&self) -> bool {
        self.audio_bitrate.is_some()
            || self.audio_stereo.is_some()
            || self.audio_dtx.is_some()
            || self.audio_red.is_some()
            || self.user_limit.is_some()
    }

    /// Prüft die Ton-Felder; sie sind nur bei Sprachkanälen erlaubt.
    fn validate_audio(&self, kind: &str) -> ApiResult<()> {
        if !self.touches_audio() {
            return Ok(());
        }
        if kind != "voice" {
            return Err(ApiError::BadRequest("not_a_voice_channel"));
        }
        if self.audio_bitrate.is_some_and(|b| !AUDIO_BITRATE_RANGE.contains(&b)) {
            return Err(ApiError::BadRequest("invalid_bitrate"));
        }
        if self.user_limit.flatten().is_some_and(|l| !USER_LIMIT_RANGE.contains(&l)) {
            return Err(ApiError::BadRequest("invalid_user_limit"));
        }
        Ok(())
    }

    /// Geänderte Ton-Felder und Kategorie fürs Audit-Log.
    fn audit(&self) -> serde_json::Value {
        let mut changes = serde_json::Map::new();
        if let Some(c) = self.category_id {
            changes.insert("category_id".into(), json!(c));
        }
        if let Some(b) = self.audio_bitrate {
            changes.insert("audio_bitrate".into(), json!(b));
        }
        if let Some(v) = self.audio_stereo {
            changes.insert("audio_stereo".into(), json!(v));
        }
        if let Some(v) = self.audio_dtx {
            changes.insert("audio_dtx".into(), json!(v));
        }
        if let Some(v) = self.audio_red {
            changes.insert("audio_red".into(), json!(v));
        }
        if let Some(l) = self.user_limit {
            changes.insert("user_limit".into(), json!(l));
        }
        serde_json::Value::Object(changes)
    }
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Channel>> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::MANAGE_CHANNELS)?;
    let current = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;

    let name = body.name.as_deref().map(|n| normalize_channel_name(n, &current.kind)).transpose()?;
    let topic = body.topic.clone().map(normalize_topic).transpose()?;
    body.validate_audio(&current.kind)?;
    if let Some(Some(category_id)) = body.category_id {
        categories::require_in_community(&state.db, category_id, ctx.community()?).await?;
    }
    let category_changed = body.category_id.is_some_and(|c| c != current.category_id);

    let mut tx = state.db.begin().await?;
    let channel: Channel = sqlx::query_as(concat!(
        "UPDATE channels SET
            name = COALESCE($2, name),
            topic = CASE WHEN $3 THEN $4 ELSE topic END,
            position = COALESCE($5, position),
            audio_bitrate = COALESCE($6, audio_bitrate),
            audio_stereo = COALESCE($7, audio_stereo),
            audio_dtx = COALESCE($8, audio_dtx),
            audio_red = COALESCE($9, audio_red),
            user_limit = CASE WHEN $10 THEN $11 ELSE user_limit END,
            category_id = CASE WHEN $12 THEN $13 ELSE category_id END
         WHERE id = $1
         RETURNING ",
        crate::channel_columns!()
    ))
    .bind(channel_id)
    .bind(&name)
    .bind(topic.is_some())
    .bind(topic.flatten())
    .bind(body.position)
    .bind(body.audio_bitrate)
    .bind(body.audio_stereo)
    .bind(body.audio_dtx)
    .bind(body.audio_red)
    .bind(body.user_limit.is_some())
    .bind(body.user_limit.flatten())
    .bind(body.category_id.is_some())
    .bind(body.category_id.flatten())
    .fetch_one(&mut *tx)
    .await?;
    audit::record(&mut *tx, ctx.community()?, user.user_id, "channel.update", Some(channel_id), body.audit())
        .await?;
    tx.commit().await?;

    let scope = events::channel(ctx.community_id, channel_id);
    if category_changed {
        // Welche Kategorien ein Mitglied sieht, hängt an seinen sichtbaren Kanälen (siehe `categories`).
        state.events.publish_permissions(scope, "CHANNEL_UPDATE", &channel);
    } else {
        state.events.publish(scope, "CHANNEL_UPDATE", &channel);
    }
    Ok(Json(channel))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::MANAGE_CHANNELS)?;
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM channels WHERE id = $1").bind(channel_id).execute(&mut *tx).await?;
    audit::record(
        &mut *tx,
        ctx.community()?,
        user.user_id,
        "channel.delete",
        Some(channel_id),
        json!({ "name": channel.name }),
    )
    .await?;
    tx.commit().await?;

    if channel.kind == "voice" {
        voice::close_rooms(&state, vec![channel_id]);
    }
    // An die ganze Community: wer den Kanal sehen konnte, ist nach dem Löschen nicht mehr prüfbar.
    state.events.publish_permissions(
        events::community(ctx.community()?),
        "CHANNEL_DELETE",
        json!({ "id": channel_id, "community_id": ctx.community_id }),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct OverrideInfo {
    role_id: Uuid,
    allow: Permissions,
    deny: Permissions,
}

/// Alle Overrides eines Kanals (für die Berechtigungs-Ansicht).
pub async fn list_overrides(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<Json<Vec<OverrideInfo>>> {
    let (_, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::MANAGE_ROLES)?;
    let overrides: Vec<OverrideInfo> = sqlx::query_as(
        "SELECT o.role_id, o.allow, o.deny FROM channel_overrides o
         JOIN roles r ON r.id = o.role_id
         WHERE o.channel_id = $1 ORDER BY r.is_default DESC, r.position, r.id",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(overrides))
}

#[derive(Deserialize)]
pub struct OverrideBody {
    allow: i64,
    deny: i64,
}

pub async fn put_override(
    State(state): State<AppState>,
    user: AuthUser,
    Path((channel_id, role_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<OverrideBody>,
) -> ApiResult<StatusCode> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::MANAGE_ROLES)?;
    models::role_by_id(&state.db, role_id)
        .await?
        .filter(|r| Some(r.community_id) == ctx.community_id)
        .ok_or_else(ApiError::not_found)?;

    let allow = Permissions(body.allow).sanitized();
    let deny = Permissions(body.deny).sanitized();
    // Nur Rechte freigeben oder entziehen, die man selbst hat.
    permissions::require(perms, allow | deny)?;

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO channel_overrides (channel_id, role_id, allow, deny) VALUES ($1, $2, $3, $4)
         ON CONFLICT (channel_id, role_id) DO UPDATE SET allow = $3, deny = $4",
    )
    .bind(channel_id)
    .bind(role_id)
    .bind(allow.0)
    .bind(deny.0)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        ctx.community()?,
        user.user_id,
        "channel.override_set",
        Some(channel_id),
        json!({ "role_id": role_id, "allow": allow, "deny": deny }),
    )
    .await?;
    tx.commit().await?;

    permissions_changed(&state, ctx.community()?);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_override(
    State(state): State<AppState>,
    user: AuthUser,
    Path((channel_id, role_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::MANAGE_ROLES)?;

    let mut tx = state.db.begin().await?;
    let deleted = sqlx::query("DELETE FROM channel_overrides WHERE channel_id = $1 AND role_id = $2")
        .bind(channel_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    audit::record(
        &mut *tx,
        ctx.community()?,
        user.user_id,
        "channel.override_delete",
        Some(channel_id),
        json!({ "role_id": role_id }),
    )
    .await?;
    tx.commit().await?;

    permissions_changed(&state, ctx.community()?);
    Ok(StatusCode::NO_CONTENT)
}

/// Alle Mitglieder sollen ihre Rechte/Kanalliste neu laden.
pub fn permissions_changed(state: &AppState, community_id: Uuid) {
    state.events.publish_permissions(
        events::community(community_id),
        "PERMISSIONS_CHANGED",
        json!({ "community_id": community_id }),
    );
    // Wer VIEW_CHANNEL/CONNECT verloren hat, fliegt aus dem Voice-Kanal.
    voice::recheck_community(state, community_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_channel_names_are_normalized() {
        assert_eq!(normalize_channel_name("  Off Topic  Chat ", "text").unwrap(), "off-topic-chat");
        assert_eq!(normalize_channel_name("Gaming Lounge", "voice").unwrap(), "Gaming Lounge");
        assert!(normalize_channel_name("   ", "text").is_err());
        assert!(normalize_channel_name(&"x".repeat(65), "voice").is_err());
    }
}
