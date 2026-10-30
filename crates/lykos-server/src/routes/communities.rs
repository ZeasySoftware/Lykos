use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    admin,
    audit::{self, AuditEntry},
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::{self, Control, Scope},
    instance_settings,
    models::{self, Channel, Community, CommunityRow, Member, Role, double_option},
    permissions::{self, Permissions},
    routes::{customization, read_states, roles},
    state::AppState,
    voice,
};

#[derive(Serialize)]
pub struct CommunityDetail {
    community: Community,
    channels: Vec<Channel>,
    roles: Vec<Role>,
    permissions: Permissions,
    channel_permissions: HashMap<Uuid, Permissions>,
    /// Lesestand je sichtbarem Textkanal (channel_id → last_read_message_id).
    read_states: HashMap<Uuid, Uuid>,
    /// Neueste Nachricht je sichtbarem Textkanal (channel_id → message_id).
    last_message_ids: HashMap<Uuid, Uuid>,
    /// Ungelesene Erwähnungen des Nutzers je sichtbarem Textkanal (nur Kanäle mit mindestens einer).
    mention_counts: HashMap<Uuid, i64>,
}

/// Community mit allen für den Nutzer sichtbaren Kanälen und seinen Rechten.
pub async fn detail(state: &AppState, user_id: Uuid, community_id: Uuid) -> ApiResult<CommunityDetail> {
    let ctx = permissions::member_context(&state.db, user_id, community_id).await?;
    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    let overrides = permissions::community_overrides(&state.db, community_id).await?;

    let mut channel_permissions = HashMap::new();
    let channels = models::channels_of(&state.db, community_id)
        .await?
        .into_iter()
        .filter(|c| {
            let perms = ctx.in_channel(overrides.get(&c.id).map_or(&[], Vec::as_slice));
            channel_permissions.insert(c.id, perms);
            perms.contains(Permissions::VIEW_CHANNEL)
        })
        .collect::<Vec<_>>();
    channel_permissions.retain(|id, _| channels.iter().any(|c| c.id == *id));

    let (mut read_states, mut last_message_ids) =
        read_states::unread_info(state, user_id, community_id).await?;
    read_states.retain(|id, _| channel_permissions.contains_key(id));
    last_message_ids.retain(|id, _| channel_permissions.contains_key(id));
    let mut mention_counts = read_states::mention_counts(state, user_id, community_id).await?;
    mention_counts.retain(|id, _| channel_permissions.contains_key(id));

    Ok(CommunityDetail {
        community,
        channels,
        roles: models::roles_of(&state.db, community_id).await?,
        permissions: ctx.base(),
        channel_permissions,
        read_states,
        last_message_ids,
        mention_counts,
    })
}

pub fn validate_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if (1..=100).contains(&name.chars().count()) {
        Ok(name.to_owned())
    } else {
        Err(ApiError::BadRequest("invalid_name"))
    }
}

pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<Community>>> {
    let communities: Vec<CommunityRow> = sqlx::query_as(concat!(
        "SELECT ",
        crate::community_columns!(),
        " FROM communities c JOIN members m ON m.community_id = c.id
         WHERE m.user_id = $1 ORDER BY m.joined_at"
    ))
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(communities.into_iter().map(Community::from).collect()))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<CommunityDetail>)> {
    let name = validate_name(&body.name)?;
    // Instanz-Einstellung: Communities anlegen ggf. nur für Instanz-Admins.
    if !instance_settings::load(&state.db, &state.config).await?.allow_community_creation
        && !admin::is_instance_admin(&state.db, user.user_id).await?
    {
        return Err(ApiError::Forbidden("community_creation_disabled"));
    }
    let community_id = Uuid::now_v7();

    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO communities (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(community_id)
        .bind(&name)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO roles (id, community_id, name, permissions, is_default) VALUES ($1, $2, '@alle', $3, true)",
    )
    .bind(Uuid::now_v7())
    .bind(community_id)
    .bind(Permissions::DEFAULT_ROLE.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO members (community_id, user_id) VALUES ($1, $2)")
        .bind(community_id)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    for (name, kind) in [("allgemein", "text"), ("Sprachkanal", "voice")] {
        sqlx::query("INSERT INTO channels (id, community_id, name, kind) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::now_v7())
            .bind(community_id)
            .bind(name)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
    }
    audit::record(&mut *tx, community_id, user.user_id, "community.create", None, json!({ "name": name }))
        .await?;
    tx.commit().await?;

    state.events.control(Scope::User(user.user_id), Control::Joined { user_id: user.user_id, community_id });
    Ok((StatusCode::CREATED, Json(detail(&state, user.user_id, community_id).await?)))
}

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<CommunityDetail>> {
    Ok(Json(detail(&state, user.user_id, community_id).await?))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    message_retention_days: Option<Option<i32>>,
    /// Speicher-Kontingent für Anhänge in MB (`null` = unbegrenzt).
    #[serde(default, deserialize_with = "double_option")]
    upload_quota_mb: Option<Option<i32>>,
    /// Kurzbeschreibung (max. 300 Zeichen, `null`/leer = entfernen).
    #[serde(default, deserialize_with = "double_option")]
    description: Option<Option<String>>,
    /// Akzentfarbe 0xRRGGBB (`null` = Standard).
    #[serde(default, deserialize_with = "double_option")]
    accent_color: Option<Option<i32>>,
    /// Textkanal der Community für Beitritts-Meldungen (`null` = keine).
    #[serde(default, deserialize_with = "double_option")]
    system_channel_id: Option<Option<Uuid>>,
}

/// Obergrenze für das Community-Kontingent (≈ 10 TB) – schützt nur vor Unsinn.
const MAX_UPLOAD_QUOTA_MB: i32 = 10_000_000;

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Community>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;

    let name = body.name.as_deref().map(validate_name).transpose()?;
    if let Some(Some(days)) = body.message_retention_days
        && !(1..=3650).contains(&days)
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    if let Some(Some(mb)) = body.upload_quota_mb
        && !(1..=MAX_UPLOAD_QUOTA_MB).contains(&mb)
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let description = body
        .description
        .map(|d| crate::auth::normalize_text(d, customization::DESCRIPTION_MAX_CHARS, "invalid_description"))
        .transpose()?;
    if let Some(color) = body.accent_color {
        customization::validate_color(color)?;
    }
    if let Some(Some(channel_id)) = body.system_channel_id {
        let valid = models::channel_by_id(&state.db, channel_id)
            .await?
            .is_some_and(|c| c.community_id == Some(community_id) && c.kind == "text");
        if !valid {
            return Err(ApiError::BadRequest("invalid_channel"));
        }
    }

    let mut tx = state.db.begin().await?;
    // Darstellung: Feld nur setzen, wenn es im Body vorkam ($n = true).
    sqlx::query(
        "UPDATE communities SET
            description       = CASE WHEN $2 THEN $3 ELSE description END,
            accent_color      = CASE WHEN $4 THEN $5 ELSE accent_color END,
            system_channel_id = CASE WHEN $6 THEN $7 ELSE system_channel_id END
         WHERE id = $1",
    )
    .bind(community_id)
    .bind(description.is_some())
    .bind(description.clone().flatten())
    .bind(body.accent_color.is_some())
    .bind(body.accent_color.flatten())
    .bind(body.system_channel_id.is_some())
    .bind(body.system_channel_id.flatten())
    .execute(&mut *tx)
    .await?;
    if let Some(name) = &name {
        sqlx::query("UPDATE communities SET name = $2 WHERE id = $1")
            .bind(community_id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(days) = body.message_retention_days {
        sqlx::query("UPDATE communities SET message_retention_days = $2 WHERE id = $1")
            .bind(community_id)
            .bind(days)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(mb) = body.upload_quota_mb {
        sqlx::query("UPDATE communities SET upload_quota_mb = $2 WHERE id = $1")
            .bind(community_id)
            .bind(mb)
            .execute(&mut *tx)
            .await?;
    }
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "community.update",
        None,
        json!({
            "name": name,
            "message_retention_days": body.message_retention_days,
            "upload_quota_mb": body.upload_quota_mb,
            "description": description,
            "accent_color": body.accent_color,
            "system_channel_id": body.system_channel_id,
        }),
    )
    .await?;
    tx.commit().await?;

    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    state.events.publish(events::community(community_id), "COMMUNITY_UPDATE", &community);
    Ok(Json(community))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    if !ctx.is_owner {
        return Err(ApiError::forbidden());
    }
    delete_community(&state, community_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Löscht eine Community samt Inhalt, schließt ihre Voice-Räume und meldet `COMMUNITY_DELETE`
/// (Besitzer oder Instanz-Admin; Berechtigung prüft der Aufrufer).
pub async fn delete_community(state: &AppState, community_id: Uuid) -> ApiResult<()> {
    let voice_rooms = voice::active_rooms(state, community_id).await?;
    // Icon und Banner gleich mitlöschen (Anhänge holt der Hintergrund-Job).
    let images: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("DELETE FROM communities WHERE id = $1 RETURNING icon_id, banner_id")
            .bind(community_id)
            .fetch_optional(&state.db)
            .await?;
    if let Some((icon, banner)) = images {
        let files: Vec<Uuid> = icon.into_iter().chain(banner).collect();
        crate::attachments::delete_files(state.storage.as_ref(), &files).await;
    }
    voice::close_rooms(state, voice_rooms);
    state.events.publish(events::community(community_id), "COMMUNITY_DELETE", json!({ "id": community_id }));
    Ok(())
}

pub async fn members(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    permissions::member_context(&state.db, user.user_id, community_id).await?;
    Ok(Json(models::members_of(&state.db, community_id).await?))
}

pub async fn remove_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    let is_self = target_id == user.user_id;

    if target_id == community.owner_id {
        return Err(if is_self { ApiError::Conflict("owner_cannot_leave") } else { ApiError::forbidden() });
    }
    if !is_self {
        permissions::require(ctx.base(), Permissions::KICK_MEMBERS)?;
        // Rollenhierarchie wie beim Nickname: nur Mitglieder, deren höchste Rolle unter der eigenen steht.
        roles::require_outranks_member(&state.db, &ctx, target_id).await?;
    }

    let mut tx = state.db.begin().await?;
    let removed = sqlx::query("DELETE FROM members WHERE community_id = $1 AND user_id = $2")
        .bind(community_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(ApiError::not_found());
    }
    let action = if is_self { "member.leave" } else { "member.kick" };
    audit::record(&mut *tx, community_id, user.user_id, action, Some(target_id), json!({})).await?;
    tx.commit().await?;

    state.events.publish(
        events::community(community_id),
        "MEMBER_REMOVE",
        json!({ "community_id": community_id, "user_id": target_id }),
    );
    state.events.control(Scope::User(target_id), Control::Left { user_id: target_id, community_id });
    voice::member_removed(&state, community_id, target_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Rolle zuweisen (`add = true`) oder entziehen.
async fn set_member_role(
    state: &AppState,
    user: AuthUser,
    community_id: Uuid,
    target_id: Uuid,
    role_id: Uuid,
    add: bool,
) -> ApiResult<StatusCode> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_ROLES)?;

    let role = models::role_by_id(&state.db, role_id)
        .await?
        .filter(|r| r.community_id == community_id && !r.is_default)
        .ok_or_else(ApiError::not_found)?;
    // Niemand vergibt Rechte, die er selbst nicht hat – und nur Rollen unterhalb der eigenen höchsten.
    permissions::require(ctx.base(), role.permissions)?;
    roles::require_outranks(&state.db, &ctx, &role).await?;

    let mut tx = state.db.begin().await?;
    let changed = if add {
        sqlx::query(
            "INSERT INTO member_roles (community_id, user_id, role_id)
             SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM members WHERE community_id = $1 AND user_id = $2)
             ON CONFLICT DO NOTHING",
        )
    } else {
        sqlx::query("DELETE FROM member_roles WHERE community_id = $1 AND user_id = $2 AND role_id = $3")
    }
    .bind(community_id)
    .bind(target_id)
    .bind(role_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if changed > 0 {
        let action = if add { "member.role_add" } else { "member.role_remove" };
        audit::record(
            &mut *tx,
            community_id,
            user.user_id,
            action,
            Some(target_id),
            json!({ "role_id": role_id }),
        )
        .await?;
    }
    tx.commit().await?;

    if changed > 0
        && let Some(member) = models::member(&state.db, community_id, target_id).await?
    {
        state.events.publish_permissions(
            events::community(community_id),
            "MEMBER_UPDATE",
            json!({ "community_id": community_id, "member": member }),
        );
        state.events.publish(
            Scope::User(target_id),
            "PERMISSIONS_CHANGED",
            json!({ "community_id": community_id }),
        );
        voice::recheck_community(state, community_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_member_role(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target_id, role_id)): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    set_member_role(&state, user, community_id, target_id, role_id, true).await
}

pub async fn remove_member_role(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target_id, role_id)): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    set_member_role(&state, user, community_id, target_id, role_id, false).await
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub before: Option<Uuid>,
    pub limit: Option<i64>,
}

impl PageQuery {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 100)
    }
}

pub async fn audit_log(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Vec<AuditEntry>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::VIEW_AUDIT_LOG)?;
    Ok(Json(audit::page(&state.db, community_id, q.before, q.limit()).await?))
}

#[derive(Deserialize)]
pub struct UpdateMemberBody {
    #[serde(default, deserialize_with = "double_option")]
    nickname: Option<Option<String>>,
}

/// `PATCH /communities/{id}/members/{user_id|@me}` – Nickname setzen (`null`/leer = entfernen).
/// Eigener Nickname: `CHANGE_NICKNAME` (oder `MANAGE_NICKNAMES`); fremder: `MANAGE_NICKNAMES`,
/// den des Besitzers ändert nur der Besitzer selbst. Fremde Änderungen landen im Audit-Log.
pub async fn update_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, target)): Path<(Uuid, String)>,
    Json(body): Json<UpdateMemberBody>,
) -> ApiResult<Json<Member>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    let target_id = if target == "@me" {
        user.user_id
    } else {
        Uuid::parse_str(&target).map_err(|_| ApiError::not_found())?
    };
    let is_self = target_id == user.user_id;
    let base = ctx.base();
    if is_self {
        if !base.contains(Permissions::CHANGE_NICKNAME) && !base.contains(Permissions::MANAGE_NICKNAMES) {
            return Err(ApiError::forbidden());
        }
    } else {
        permissions::require(base, Permissions::MANAGE_NICKNAMES)?;
        let community =
            models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
        if target_id == community.owner_id {
            return Err(ApiError::forbidden());
        }
        // Rollenhierarchie: nur Mitglieder, deren höchste Rolle unter der eigenen höchsten steht.
        roles::require_outranks_member(&state.db, &ctx, target_id).await?;
    }

    let Some(nickname) = body.nickname else {
        // Nichts zu ändern.
        return models::member(&state.db, community_id, target_id)
            .await?
            .map(Json)
            .ok_or_else(ApiError::not_found);
    };
    let nickname = crate::auth::normalize_text(nickname, crate::auth::NICKNAME_MAX_CHARS, "invalid_name")?;
    if nickname.as_deref().is_some_and(|n| n.contains('\n')) {
        return Err(ApiError::BadRequest("invalid_name"));
    }

    let mut tx = state.db.begin().await?;
    let updated = sqlx::query("UPDATE members SET nickname = $3 WHERE community_id = $1 AND user_id = $2")
        .bind(community_id)
        .bind(target_id)
        .bind(&nickname)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    if !is_self {
        audit::record(
            &mut *tx,
            community_id,
            user.user_id,
            "member.nickname",
            Some(target_id),
            json!({ "nickname": nickname }),
        )
        .await?;
    }
    tx.commit().await?;

    let member = models::member(&state.db, community_id, target_id).await?.ok_or_else(ApiError::not_found)?;
    state.events.publish(
        events::community(community_id),
        "MEMBER_UPDATE",
        json!({ "community_id": community_id, "member": member }),
    );
    Ok(Json(member))
}

#[derive(Deserialize)]
pub struct TransferBody {
    user_id: Uuid,
}

/// `POST /communities/{id}/transfer` – Besitz an ein anderes Mitglied übertragen (nur Besitzer).
pub async fn transfer(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> ApiResult<Json<Community>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    if !ctx.is_owner {
        return Err(ApiError::forbidden());
    }
    if body.user_id == user.user_id {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    Ok(Json(transfer_ownership(&state, community_id, user.user_id, body.user_id, user.user_id).await?))
}

/// Überträgt den Besitz von `from` an das Mitglied `to` (sonst `404`) und meldet die Änderung.
/// `actor` ist, wer es auslöst (Besitzer oder Instanz-Admin); Berechtigung prüft der Aufrufer.
pub async fn transfer_ownership(
    state: &AppState,
    community_id: Uuid,
    from: Uuid,
    to: Uuid,
    actor: Uuid,
) -> ApiResult<Community> {
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query(
        "UPDATE communities SET owner_id = $2
         WHERE id = $1 AND owner_id = $3
           AND EXISTS (SELECT 1 FROM members WHERE community_id = $1 AND user_id = $2)",
    )
    .bind(community_id)
    .bind(to)
    .bind(from)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        // Ziel ist kein Mitglied.
        return Err(ApiError::not_found());
    }
    audit::record(&mut *tx, community_id, actor, "community.transfer", Some(to), json!({ "from": from }))
        .await?;
    tx.commit().await?;

    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    state.events.publish_permissions(events::community(community_id), "COMMUNITY_UPDATE", &community);
    for id in [from, to] {
        state.events.publish(Scope::User(id), "PERMISSIONS_CHANGED", json!({ "community_id": community_id }));
    }
    voice::recheck_community(state, community_id);
    Ok(community)
}
