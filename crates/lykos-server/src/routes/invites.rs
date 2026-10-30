use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgConnection, Postgres, Transaction};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    audit,
    auth::{self, AuthUser},
    error::{ApiError, ApiResult},
    events::{self, Control, Scope},
    instance_settings,
    models::{self, Invite},
    permissions::{self, Permissions},
    routes::{
        bans,
        communities::{self, CommunityDetail},
        customization,
    },
    state::AppState,
};

macro_rules! invite_columns {
    () => {
        "code, community_id, max_uses, uses, expires_at, created_at"
    };
}

#[derive(Deserialize)]
pub struct CreateInviteBody {
    max_uses: Option<i32>,
    expires_in_hours: Option<i64>,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<CreateInviteBody>,
) -> ApiResult<(StatusCode, Json<Invite>)> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::CREATE_INVITES)?;

    if body.max_uses.is_some_and(|n| !(1..=10_000).contains(&n))
        || body.expires_in_hours.is_some_and(|h| !(1..=24 * 365).contains(&h))
    {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let expires_at = body.expires_in_hours.map(|h| OffsetDateTime::now_utc() + Duration::hours(h));

    let mut tx = state.db.begin().await?;
    let invite: Invite = sqlx::query_as(concat!(
        "INSERT INTO invites (code, community_id, created_by, max_uses, expires_at)
         VALUES ($1, $2, $3, $4, $5) RETURNING ",
        invite_columns!()
    ))
    .bind(auth::random_token(6))
    .bind(community_id)
    .bind(user.user_id)
    .bind(body.max_uses)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "invite.create",
        None,
        json!({ "code": invite.code }),
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(invite)))
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Invite>>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;

    let invites: Vec<Invite> = sqlx::query_as(concat!(
        "SELECT ",
        invite_columns!(),
        " FROM invites WHERE community_id = $1
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR uses < max_uses)
         ORDER BY created_at DESC"
    ))
    .bind(community_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(invites))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    let row: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT community_id, created_by FROM invites WHERE code = $1")
            .bind(&code)
            .fetch_optional(&state.db)
            .await?;
    let (community_id, created_by) = row.ok_or(ApiError::NotFound("invalid_invite"))?;

    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    if created_by != Some(user.user_id) {
        permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;
    }

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM invites WHERE code = $1").bind(&code).execute(&mut *tx).await?;
    audit::record(&mut *tx, community_id, user.user_id, "invite.delete", None, json!({ "code": code }))
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct InvitePreview {
    code: String,
    /// `community` = Einladung in eine Community, `instance` = Registrierungs-Einladung der Instanz
    /// (optional mit automatischem Beitritt zu `community`).
    kind: &'static str,
    /// Name der Instanz (für „Du wurdest zu … eingeladen“).
    instance_name: String,
    /// Bei Instanz-Einladungen ohne Community `null`.
    community: Option<CommunityRef>,
    member_count: i64,
}

/// Öffentliche Angaben zur Community auf der Einladungsseite. Bild-URLs laufen über den Einladungscode,
/// damit Nicht-Mitglieder Icon und Banner sehen, ohne dass die Bilder allgemein öffentlich sind.
#[derive(Serialize)]
struct CommunityRef {
    id: Uuid,
    name: String,
    icon_url: Option<String>,
    banner_url: Option<String>,
    description: Option<String>,
    accent_color: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct PreviewRow {
    id: Uuid,
    name: String,
    icon_id: Option<Uuid>,
    banner_id: Option<Uuid>,
    description: Option<String>,
    accent_color: Option<i32>,
    member_count: i64,
}

/// Gültigkeitsbedingung einer Instanz-Einladung (Alias `i`): nicht widerrufen, nicht abgelaufen, nicht aufgebraucht.
macro_rules! instance_invite_valid {
    () => {
        "i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > now())
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)"
    };
}

/// Öffentlich: zeigt, wohin eine Einladung führt (Community- oder Instanz-Einladung).
pub async fn preview(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<Json<InvitePreview>> {
    let instance_name = instance_settings::load(&state.db, &state.config).await?.instance_name;
    let row: Option<PreviewRow> = sqlx::query_as(
        "SELECT c.id, c.name, c.icon_id, c.banner_id, c.description, c.accent_color,
                (SELECT count(*) FROM members m WHERE m.community_id = c.id) AS member_count
         FROM invites i JOIN communities c ON c.id = i.community_id
         WHERE i.code = $1
           AND (i.expires_at IS NULL OR i.expires_at > now())
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)",
    )
    .bind(&code)
    .fetch_optional(&state.db)
    .await?;
    let community_ref = |r: PreviewRow| CommunityRef {
        id: r.id,
        name: r.name,
        icon_url: customization::invite_image_url(&code, "icon", r.icon_id),
        banner_url: customization::invite_image_url(&code, "banner", r.banner_id),
        description: r.description,
        accent_color: r.accent_color,
    };
    if let Some(r) = row {
        let member_count = r.member_count;
        let community = Some(community_ref(r));
        return Ok(Json(InvitePreview { kind: "community", instance_name, community, member_count, code }));
    }

    // Instanz-Einladung, ggf. mit automatischem Beitritt zu einer Community.
    let invite: Option<(Option<Uuid>,)> = sqlx::query_as(concat!(
        "SELECT i.community_id FROM instance_invites i WHERE i.code = $1 AND ",
        instance_invite_valid!()
    ))
    .bind(&code)
    .fetch_optional(&state.db)
    .await?;
    let (community_id,) = invite.ok_or(ApiError::NotFound("invalid_invite"))?;
    let row: Option<PreviewRow> = match community_id {
        Some(id) => {
            sqlx::query_as(
                "SELECT c.id, c.name, c.icon_id, c.banner_id, c.description, c.accent_color,
                        (SELECT count(*) FROM members m WHERE m.community_id = c.id) AS member_count
                 FROM communities c WHERE c.id = $1",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await?
        }
        None => None,
    };
    let member_count = row.as_ref().map_or(0, |r| r.member_count);
    let community = row.map(community_ref);
    Ok(Json(InvitePreview { kind: "instance", instance_name, community, member_count, code }))
}

pub async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(code): Path<String>,
) -> ApiResult<Json<CommunityDetail>> {
    let mut tx = state.db.begin().await?;
    let community_id = match join_via_invite(&mut tx, &code, user.user_id).await {
        // Instanz-Einladungen gelten nur für die Registrierung.
        Err(ApiError::NotFound(_)) if is_instance_invite(&mut tx, &code).await? => {
            return Err(ApiError::Conflict("instance_invite"));
        }
        other => other?,
    };
    tx.commit().await?;

    announce_join(&state, community_id, user.user_id).await?;
    Ok(Json(communities::detail(&state, user.user_id, community_id).await?))
}

async fn is_instance_invite(conn: &mut PgConnection, code: &str) -> ApiResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM instance_invites WHERE code = $1)")
        .bind(code)
        .fetch_one(conn)
        .await?)
}

/// Löst bei der Registrierung eine Einladung ein – Community-Einladung (Beitritt) oder Instanz-Einladung
/// (Nutzung zählen, Registrierung vermerken, ggf. automatischer Beitritt). Läuft in der Transaktion des
/// Aufrufers, nachdem der Nutzer angelegt wurde; ungültige Codes → `404 invalid_invite` (Aufrufer rollt
/// zurück). Liefert die beigetretene Community – nach dem Commit `announce_join` aufrufen.
///
/// Gemeinsamer Weg für alle Registrierungsarten (Passwort, später OIDC).
pub async fn redeem_for_registration(
    tx: &mut Transaction<'static, Postgres>,
    code: &str,
    user_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    if !is_instance_invite(tx, code).await? {
        return join_via_invite(tx, code, user_id).await.map(Some);
    }

    let conn: &mut PgConnection = tx;
    let invite: Option<(Option<Uuid>,)> = sqlx::query_as(concat!(
        "SELECT i.community_id FROM instance_invites i WHERE i.code = $1 AND ",
        instance_invite_valid!(),
        " FOR UPDATE"
    ))
    .bind(code)
    .fetch_optional(&mut *conn)
    .await?;
    let (community_id,) = invite.ok_or(ApiError::NotFound("invalid_invite"))?;

    sqlx::query("UPDATE instance_invites SET uses = uses + 1 WHERE code = $1")
        .bind(code)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO instance_invite_uses (code, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(code)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

    // Automatischer Beitritt – die Community kann inzwischen gelöscht sein (dann `community_id` NULL).
    let Some(community_id) = community_id else { return Ok(None) };
    let inserted =
        sqlx::query("INSERT INTO members (community_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(community_id)
            .bind(user_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if inserted == 0 {
        return Ok(None);
    }
    audit::record(
        &mut *conn,
        community_id,
        user_id,
        "member.join",
        Some(user_id),
        json!({ "instance_invite": true }),
    )
    .await?;
    Ok(Some(community_id))
}

/// Löst eine Einladung ein und macht den Nutzer zum Mitglied. Läuft in der Transaktion des Aufrufers.
pub async fn join_via_invite(
    tx: &mut Transaction<'static, Postgres>,
    code: &str,
    user_id: Uuid,
) -> ApiResult<Uuid> {
    let conn: &mut PgConnection = tx;
    let invite: Option<(Uuid,)> = sqlx::query_as(
        "SELECT community_id FROM invites WHERE code = $1
           AND (expires_at IS NULL OR expires_at > now())
           AND (max_uses IS NULL OR uses < max_uses)
         FOR UPDATE",
    )
    .bind(code)
    .fetch_optional(&mut *conn)
    .await?;
    let (community_id,) = invite.ok_or(ApiError::NotFound("invalid_invite"))?;
    // Gebannte kommen über keine Einladung zurück, bis der Bann aufgehoben ist.
    if bans::is_banned(&mut *conn, community_id, user_id).await? {
        return Err(ApiError::Forbidden("banned"));
    }

    let inserted =
        sqlx::query("INSERT INTO members (community_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(community_id)
            .bind(user_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict("already_member"));
    }

    sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = $1").bind(code).execute(&mut *conn).await?;
    audit::record(&mut *conn, community_id, user_id, "member.join", Some(user_id), json!({ "invite": code }))
        .await?;
    Ok(community_id)
}

/// Meldet den Beitritt an Gateway-Verbindungen (nach dem Commit aufrufen).
pub async fn announce_join(state: &AppState, community_id: Uuid, user_id: Uuid) -> ApiResult<()> {
    state.events.control(Scope::User(user_id), Control::Joined { user_id, community_id });
    if let Some(member) = models::member(&state.db, community_id, user_id).await? {
        state.events.publish(
            events::community(community_id),
            "MEMBER_ADD",
            json!({ "community_id": community_id, "member": member }),
        );
    }
    // Der Beitritt ist schon gespeichert – eine fehlgeschlagene Systemnachricht soll ihn nicht scheitern lassen.
    if let Err(e) = customization::post_join_message(state, community_id, user_id).await {
        tracing::warn!(error = %e, %community_id, "Beitritts-Meldung nicht gesendet");
    }
    Ok(())
}
