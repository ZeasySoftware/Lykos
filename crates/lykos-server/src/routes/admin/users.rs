//! Nutzerverwaltung: Liste/Detail, sperren, Sitzungen beenden, Admin-Recht, Reset-Link, Konto löschen.
//!
//! Bewusst nicht enthalten: Nachrichteninhalte, IP-Adressen, genaue Aktivitätszeiten. „Zuletzt aktiv“ ist
//! nur der Tag der jüngsten Session-Nutzung (aus `sessions.last_used_at`, keine zusätzliche Erfassung).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{ListQuery, Page, base_url};
use crate::{
    accounts,
    admin::InstanceAdmin,
    audit, auth,
    error::{ApiError, ApiResult},
    events::{Control, Scope},
    models::avatar_url,
    routes::communities,
    state::AppState,
    voice,
};

/// Gültigkeit eines vom Admin erzeugten Reset-Links.
pub const ADMIN_RESET_TTL: Duration = Duration::hours(24);

#[derive(Serialize, FromRow)]
pub struct AdminUser {
    id: Uuid,
    username: String,
    display_name: Option<String>,
    #[sqlx(skip)]
    avatar_url: Option<String>,
    #[serde(skip)]
    avatar_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// Tag der letzten Aktivität (`YYYY-MM-DD`, aus der zuletzt genutzten Session); `null` = unbekannt.
    last_active_on: Option<String>,
    community_count: i64,
    is_instance_admin: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    disabled_at: Option<OffsetDateTime>,
    #[serde(skip)]
    #[sqlx(default)]
    total: i64,
}

macro_rules! user_columns {
    () => {
        "u.id, u.username, u.display_name, u.avatar_id, u.created_at, u.is_instance_admin, u.disabled_at,
         to_char((SELECT max(s.last_used_at) FROM sessions s WHERE s.user_id = u.id), 'YYYY-MM-DD') AS last_active_on,
         (SELECT count(*) FROM members m WHERE m.user_id = u.id) AS community_count"
    };
}

impl AdminUser {
    fn with_avatar(mut self) -> Self {
        self.avatar_url = avatar_url(self.id, self.avatar_id);
        self
    }
}

/// `GET /admin/users?q=&page=&per_page=&filter=admins|disabled`
pub async fn list(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Page<AdminUser>>> {
    let filter = q.filter.as_deref().unwrap_or("all");
    if !matches!(filter, "all" | "admins" | "disabled") {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let rows: Vec<AdminUser> = sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        ", count(*) OVER () AS total
         FROM users u
         WHERE ($1::text IS NULL OR u.username ILIKE $1 OR u.display_name ILIKE $1)
           AND ($2 <> 'admins' OR u.is_instance_admin)
           AND ($2 <> 'disabled' OR u.disabled_at IS NOT NULL)
         ORDER BY u.created_at DESC, u.id DESC
         LIMIT $3 OFFSET $4"
    ))
    .bind(q.pattern())
    .bind(filter)
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&state.db)
    .await?;
    let total = match rows.first() {
        Some(r) => r.total,
        // Leere Seite: Gesamtzahl trotzdem liefern (z. B. Seite hinter dem Ende).
        None => 0,
    };
    Ok(Json(Page { total, items: rows.into_iter().map(AdminUser::with_avatar).collect() }))
}

#[derive(Serialize, FromRow)]
pub struct UserCommunity {
    id: Uuid,
    name: String,
    is_owner: bool,
    #[serde(with = "time::serde::rfc3339")]
    joined_at: OffsetDateTime,
}

#[derive(Serialize)]
pub struct InviteRef {
    code: String,
    note: Option<String>,
}

#[derive(Serialize)]
pub struct UserDetail {
    #[serde(flatten)]
    user: AdminUser,
    communities: Vec<UserCommunity>,
    /// Aktive Sessions (Anzahl, ohne Geräte- oder Ortsangaben).
    session_count: i64,
    /// Anzahl eigener Nachrichten (keine Inhalte).
    message_count: i64,
    /// Belegter Speicher durch eigene Anhänge.
    storage_bytes: i64,
    /// Instanz-Einladung, mit der sich der Nutzer registriert hat.
    registered_with: Option<InviteRef>,
}

async fn load_user(state: &AppState, user_id: Uuid) -> ApiResult<AdminUser> {
    let user: Option<AdminUser> =
        sqlx::query_as(concat!("SELECT ", user_columns!(), " FROM users u WHERE u.id = $1"))
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    Ok(user.ok_or_else(ApiError::not_found)?.with_avatar())
}

/// `GET /admin/users/{id}`
pub async fn detail(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<Json<UserDetail>> {
    let user = load_user(&state, user_id).await?;
    let communities: Vec<UserCommunity> = sqlx::query_as(
        "SELECT c.id, c.name, c.owner_id = m.user_id AS is_owner, m.joined_at
         FROM members m JOIN communities c ON c.id = m.community_id
         WHERE m.user_id = $1 ORDER BY m.joined_at",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    let (session_count, message_count, storage_bytes): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM sessions WHERE user_id = $1 AND expires_at > now()),
                (SELECT count(*) FROM messages WHERE author_id = $1),
                (SELECT COALESCE(sum(size_bytes), 0)::BIGINT FROM attachments WHERE uploader_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    let registered_with: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT i.code, i.note FROM instance_invite_uses iu JOIN instance_invites i ON i.code = iu.code
         WHERE iu.user_id = $1 ORDER BY iu.used_at LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(Json(UserDetail {
        user,
        communities,
        session_count,
        message_count,
        storage_bytes,
        registered_with: registered_with.map(|(code, note)| InviteRef { code, note }),
    }))
}

/// Benutzername des Ziels (für Audit-Einträge, die das Löschen überleben); `404`, falls unbekannt.
async fn username(state: &AppState, user_id: Uuid) -> ApiResult<String> {
    sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}

fn not_self(admin: &InstanceAdmin, user_id: Uuid) -> ApiResult<()> {
    if admin.user_id() == user_id { Err(ApiError::Conflict("cannot_target_self")) } else { Ok(()) }
}

/// Beendet Sessions des Nutzers (außer `keep`) und trennt die zugehörigen Gateway-Verbindungen.
async fn end_sessions(state: &AppState, user_id: Uuid, keep: Option<Uuid>) -> ApiResult<usize> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM sessions WHERE user_id = $1 AND ($2::uuid IS NULL OR id <> $2) RETURNING id",
    )
    .bind(user_id)
    .bind(keep)
    .fetch_all(&state.db)
    .await?;
    for id in &ids {
        state.events.control(Scope::Session(*id), Control::SessionRevoked);
    }
    Ok(ids.len())
}

/// Wirft den Nutzer aus seinem Voice-Kanal bzw. DM-Anruf (falls er in einem ist).
async fn kick_from_voice(state: &AppState, user_id: Uuid) -> ApiResult<()> {
    voice::disconnect_user(state, user_id).await
}

/// `POST /admin/users/{id}/disable` – sperren: Anmeldung verweigert, Sessions gelöscht, Gateway getrennt,
/// aus Voice geworfen. Nicht für das eigene Konto.
pub async fn disable(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    not_self(&admin, user_id)?;
    let username = username(&state, user_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET disabled_at = COALESCE(disabled_at, now()) WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    audit::record_instance(
        &mut *tx,
        admin.user_id(),
        "user.disable",
        Some(user_id),
        json!({ "username": username }),
    )
    .await?;
    tx.commit().await?;

    kick_from_voice(&state, user_id).await?;
    end_sessions(&state, user_id, None).await?;
    tracing::info!(%user_id, "Konto gesperrt");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/users/{id}/enable` – Sperre aufheben.
pub async fn enable(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let username = username(&state, user_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET disabled_at = NULL WHERE id = $1").bind(user_id).execute(&mut *tx).await?;
    audit::record_instance(
        &mut *tx,
        admin.user_id(),
        "user.enable",
        Some(user_id),
        json!({ "username": username }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /admin/users/{id}/sessions` – alle Sessions beenden (beim eigenen Konto außer der aktuellen).
pub async fn revoke_sessions(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let username = username(&state, user_id).await?;
    let keep = (admin.user_id() == user_id).then_some(admin.0.session_id);
    let count = end_sessions(&state, user_id, keep).await?;
    audit::record_instance(
        &state.db,
        admin.user_id(),
        "user.sessions_revoke",
        Some(user_id),
        json!({ "username": username, "count": count }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /admin/users/{id}/admin` – zum Instanz-Admin machen.
pub async fn grant_admin(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    set_admin(&state, &admin, user_id, true).await
}

/// `DELETE /admin/users/{id}/admin` – Admin-Recht entziehen (nie dem letzten aktiven Admin).
pub async fn revoke_admin(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    set_admin(&state, &admin, user_id, false).await
}

/// Advisory-Lock-Schlüssel für Änderungen an der Admin-Menge (verhindert, dass zwei Admins sich
/// gleichzeitig gegenseitig entziehen und keiner übrig bleibt).
const ADMIN_LOCK: i64 = 7002;

async fn set_admin(
    state: &AppState,
    admin: &InstanceAdmin,
    user_id: Uuid,
    value: bool,
) -> ApiResult<StatusCode> {
    let username = username(state, user_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(ADMIN_LOCK).execute(&mut *tx).await?;
    if !value {
        let others: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users WHERE is_instance_admin AND disabled_at IS NULL AND id <> $1",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if others == 0 {
            return Err(ApiError::Conflict("last_instance_admin"));
        }
    }
    sqlx::query("UPDATE users SET is_instance_admin = $2 WHERE id = $1")
        .bind(user_id)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    let action = if value { "user.admin_grant" } else { "user.admin_revoke" };
    audit::record_instance(&mut *tx, admin.user_id(), action, Some(user_id), json!({ "username": username }))
        .await?;
    tx.commit().await?;
    // Nur an den Betroffenen: Sein Client blendet die Instanz-Verwaltung ein bzw. aus.
    state.events.publish(
        Scope::User(user_id),
        "INSTANCE_ADMIN_UPDATE",
        json!({ "is_instance_admin": value }),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct ResetLink {
    /// Einmal-Link `{public_url}/reset-password#token=…` – nur in dieser Antwort im Klartext.
    url: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

/// `POST /admin/users/{id}/password-reset` – Einmal-Link zum Zurücksetzen des Passworts (24 h), den der
/// Admin dem Nutzer selbst schickt. Ältere, unbenutzte Admin-Links des Nutzers werden ungültig.
pub async fn password_reset_link(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<ResetLink>)> {
    let username = username(&state, user_id).await?;
    let token = auth::random_token(32);
    let expires_at = OffsetDateTime::now_utc() + ADMIN_RESET_TTL;

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "DELETE FROM password_reset_tokens WHERE user_id = $1 AND kind = 'admin_reset' AND used_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO password_reset_tokens (id, user_id, token_hash, kind, created_by, expires_at)
         VALUES ($1, $2, $3, 'admin_reset', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(auth::token_hash(&token))
    .bind(admin.user_id())
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    audit::record_instance(
        &mut *tx,
        admin.user_id(),
        "user.password_reset_link",
        Some(user_id),
        json!({ "username": username }),
    )
    .await?;
    tx.commit().await?;

    // Token im Fragment (nicht in Server-/Proxy-Logs oder `Referer`); ältere `?token=`-Links liest der Client weiter.
    let url = format!("{}/reset-password#token={token}", base_url(&state));
    Ok((StatusCode::CREATED, Json(ResetLink { url, expires_at })))
}

#[derive(Deserialize)]
pub struct DeleteBody {
    /// Nachrichten des Nutzers mitlöschen; sonst bleiben sie als „Gelöschter Nutzer“ stehen.
    #[serde(default)]
    delete_messages: bool,
    /// Umgang mit Communities, die der Nutzer besitzt: `transfer` = an das am längsten beigetretene andere
    /// Mitglied übertragen (Communities ohne weitere Mitglieder werden gelöscht), `delete` = alle löschen.
    /// Fehlt die Angabe und besitzt der Nutzer Communities → `409 owns_communities`.
    owned_communities: Option<String>,
}

/// `DELETE /admin/users/{id}` – Konto löschen (wie Selbstlöschung). Nicht das eigene Konto.
pub async fn delete(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Path(user_id): Path<Uuid>,
    Json(body): Json<DeleteBody>,
) -> ApiResult<StatusCode> {
    not_self(&admin, user_id)?;
    let strategy = body.owned_communities.as_deref();
    if !matches!(strategy, None | Some("transfer" | "delete")) {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let username = username(&state, user_id).await?;

    let owned: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM communities WHERE owner_id = $1 ORDER BY created_at")
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
    if !owned.is_empty() && strategy.is_none() {
        return Err(ApiError::Conflict("owns_communities"));
    }

    let mut handled = Vec::new();
    for (community_id, name) in owned {
        let heir: Option<Uuid> = if strategy == Some("transfer") {
            sqlx::query_scalar(
                "SELECT user_id FROM members WHERE community_id = $1 AND user_id <> $2
                 ORDER BY joined_at, user_id LIMIT 1",
            )
            .bind(community_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?
        } else {
            None
        };
        match heir {
            Some(heir) => {
                communities::transfer_ownership(&state, community_id, user_id, heir, admin.user_id()).await?;
                handled.push(
                    json!({ "id": community_id, "name": name, "action": "transfer", "new_owner": heir }),
                );
            }
            None => {
                communities::delete_community(&state, community_id).await?;
                handled.push(json!({ "id": community_id, "name": name, "action": "delete" }));
            }
        }
    }

    audit::record_instance(
        &state.db,
        admin.user_id(),
        "user.delete",
        Some(user_id),
        json!({ "username": username, "delete_messages": body.delete_messages, "communities": handled }),
    )
    .await?;
    accounts::delete_account(&state, user_id, body.delete_messages).await?;
    tracing::info!(%user_id, delete_messages = body.delete_messages, "Konto durch Instanz-Admin gelöscht");
    Ok(StatusCode::NO_CONTENT)
}
