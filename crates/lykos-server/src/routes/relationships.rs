//! Freundesystem: Freundschaftsanfragen, Freunde und Blockieren.
//!
//! Jede Beziehung steht aus Sicht beider Seiten in `relationships` (siehe Migration 0018): Freunde und
//! Anfragen paarweise (`friend`/`friend`, `pending_out`/`pending_in`), Blockieren nur auf der Seite des
//! Blockierenden. Wer blockiert ist, kann keine Anfragen schicken, keine DM eröffnen und in einer
//! bestehenden 1:1-DM nicht mehr schreiben (`permissions::channel_permissions`); Clients blenden seine
//! Nachrichten aus. Freunde dürfen sich DMs schreiben, auch ohne gemeinsame Community, und sehen
//! gegenseitig Profil und Präsenz.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::{Control, Scope},
    models::UserPublic,
    presence::Status,
    routes::calls,
    state::AppState,
};

/// Höchstzahl offener ausgehender Anfragen (Schutz vor Spam).
pub const MAX_PENDING_OUT: i64 = 100;

/// Art einer Beziehung aus Sicht des Nutzers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Kind {
    Friend,
    /// Eigene, noch offene Anfrage.
    PendingOut,
    /// Anfrage an einen, noch nicht beantwortet.
    PendingIn,
    Blocked,
}

/// Beziehung zu einem anderen Nutzer (`GET /relationships`, `RELATIONSHIP_ADD`).
#[derive(Debug, Clone, Serialize)]
pub struct Relationship {
    /// ID des anderen Nutzers.
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub user: UserPublic,
    /// Seit wann die Beziehung in dieser Form besteht (Anfrage gesendet, Freundschaft geschlossen, blockiert).
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct RelationshipRow {
    target_id: Uuid,
    kind: Kind,
    created_at: OffsetDateTime,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
}

impl From<RelationshipRow> for Relationship {
    fn from(r: RelationshipRow) -> Self {
        Relationship {
            id: r.target_id,
            kind: r.kind,
            user: UserPublic::new(r.target_id, r.username, r.display_name, r.avatar_id),
            since: r.created_at,
        }
    }
}

/// Beziehungen eines Nutzers (alle oder nur die zu `only`), nach Benutzername sortiert.
async fn load(db: impl PgExecutor<'_>, user_id: Uuid, only: Option<Uuid>) -> ApiResult<Vec<Relationship>> {
    let rows: Vec<RelationshipRow> = sqlx::query_as(
        "SELECT r.target_id, r.kind, r.created_at, u.username, u.display_name, u.avatar_id
         FROM relationships r JOIN users u ON u.id = r.target_id
         WHERE r.user_id = $1 AND ($2::uuid IS NULL OR r.target_id = $2)
         ORDER BY lower(u.username)",
    )
    .bind(user_id)
    .bind(only)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Relationship::from).collect())
}

/// Art der Beziehung von `user_id` zu `target_id`, falls es eine gibt.
async fn kind_of(db: impl PgExecutor<'_>, user_id: Uuid, target_id: Uuid) -> ApiResult<Option<Kind>> {
    Ok(sqlx::query_scalar("SELECT kind FROM relationships WHERE user_id = $1 AND target_id = $2")
        .bind(user_id)
        .bind(target_id)
        .fetch_optional(db)
        .await?)
}

/// IDs aller Freunde eines Nutzers.
pub async fn friend_ids(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT target_id FROM relationships WHERE user_id = $1 AND kind = 'friend'")
        .bind(user_id)
        .fetch_all(db)
        .await?)
}

/// Hat einer der beiden den anderen blockiert?
pub async fn blocked_between(db: impl PgExecutor<'_>, a: Uuid, b: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM relationships WHERE kind = 'blocked'
                        AND ((user_id = $1 AND target_id = $2) OR (user_id = $2 AND target_id = $1)))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(db)
    .await?)
}

/// Serialisiert gleichzeitige Änderungen an der Beziehung eines Nutzerpaars (bis Transaktionsende).
async fn lock_pair(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, a: Uuid, b: Uuid) -> ApiResult<()> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("relationship:{lo}:{hi}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn put_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    target_id: Uuid,
    kind: Kind,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO relationships (user_id, target_id, kind) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, target_id) DO UPDATE SET kind = EXCLUDED.kind, created_at = now()",
    )
    .bind(user_id)
    .bind(target_id)
    .bind(kind)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn delete_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    target_id: Uuid,
) -> ApiResult<()> {
    sqlx::query("DELETE FROM relationships WHERE user_id = $1 AND target_id = $2")
        .bind(user_id)
        .bind(target_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// `RELATIONSHIP_ADD` mit dem aktuellen Stand der Beziehung `user_id` → `target_id` an `user_id`.
async fn announce_add(state: &AppState, user_id: Uuid, target_id: Uuid) -> ApiResult<Option<Relationship>> {
    let rel = load(&state.db, user_id, Some(target_id)).await?.pop();
    if let Some(rel) = &rel {
        state.events.publish(Scope::User(user_id), "RELATIONSHIP_ADD", rel);
    }
    Ok(rel)
}

/// `RELATIONSHIP_REMOVE` an `user_id`: Die Beziehung zu `target_id` besteht nicht mehr.
fn announce_remove(state: &AppState, user_id: Uuid, target_id: Uuid) {
    state.events.publish(Scope::User(user_id), "RELATIONSHIP_REMOVE", json!({ "id": target_id }));
}

/// Neue Freundschaft: Verbindungen beider Seiten nehmen sich gegenseitig in die Präsenz-Verteilung auf und
/// bekommen sofort den aktuellen Status des anderen.
fn friendship_started(state: &AppState, a: Uuid, b: Uuid) {
    for (me, other) in [(a, b), (b, a)] {
        state.events.control(Scope::User(me), Control::FriendAdded { user_id: me, friend_id: other });
        let status = state.presence.visible(other);
        if status != Status::Offline {
            state.events.publish(
                Scope::User(me),
                "PRESENCE_UPDATE",
                json!({ "user_id": other, "status": status }),
            );
        }
    }
}

fn friendship_ended(state: &AppState, a: Uuid, b: Uuid) {
    for (me, other) in [(a, b), (b, a)] {
        state.events.control(Scope::User(me), Control::FriendRemoved { user_id: me, friend_id: other });
    }
}

/// Darf `from` an `to` eine Anfrage schicken? Berücksichtigt die Datenschutz-Einstellung von `to` und
/// gesperrte Konten. Blockaden prüft der Aufrufer.
async fn accepts_requests(state: &AppState, from: Uuid, to: Uuid) -> ApiResult<bool> {
    let setting: Option<String> =
        sqlx::query_scalar("SELECT friend_requests FROM users WHERE id = $1 AND disabled_at IS NULL")
            .bind(to)
            .fetch_optional(&state.db)
            .await?;
    Ok(match setting.as_deref() {
        Some("everyone") => true,
        Some("mutual_communities") => {
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM members a JOIN members b ON a.community_id = b.community_id
                            WHERE a.user_id = $1 AND b.user_id = $2)",
            )
            .bind(from)
            .bind(to)
            .fetch_one(&state.db)
            .await?
        }
        _ => false,
    })
}

/// Anfrage senden – oder, wenn der andere schon angefragt hat, annehmen.
async fn request(state: &AppState, me: Uuid, target: Uuid) -> ApiResult<(StatusCode, Relationship)> {
    if me == target {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let mut tx = state.db.begin().await?;
    lock_pair(&mut tx, me, target).await?;
    let mine = kind_of(&mut *tx, me, target).await?;
    let theirs = kind_of(&mut *tx, target, me).await?;
    match (mine, theirs) {
        (Some(Kind::Blocked), _) => return Err(ApiError::Conflict("user_blocked")),
        (_, Some(Kind::Blocked)) => return Err(ApiError::Forbidden("friend_request_not_allowed")),
        (Some(Kind::Friend), _) => return Err(ApiError::Conflict("already_friends")),
        (Some(Kind::PendingOut), _) => {
            drop(tx);
            let rel = load(&state.db, me, Some(target)).await?.pop().ok_or_else(ApiError::not_found)?;
            return Ok((StatusCode::OK, rel));
        }
        (Some(Kind::PendingIn), _) => {
            // Annehmen.
            put_row(&mut tx, me, target, Kind::Friend).await?;
            put_row(&mut tx, target, me, Kind::Friend).await?;
            tx.commit().await?;
            announce_add(state, target, me).await?;
            let rel = announce_add(state, me, target).await?.ok_or_else(ApiError::not_found)?;
            friendship_started(state, me, target);
            return Ok((StatusCode::OK, rel));
        }
        (None, _) => {}
    }
    if !accepts_requests(state, me, target).await? {
        return Err(ApiError::Forbidden("friend_request_not_allowed"));
    }
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relationships WHERE user_id = $1 AND kind = 'pending_out'")
            .bind(me)
            .fetch_one(&mut *tx)
            .await?;
    if pending >= MAX_PENDING_OUT {
        return Err(ApiError::Conflict("too_many_pending_requests"));
    }
    put_row(&mut tx, me, target, Kind::PendingOut).await?;
    put_row(&mut tx, target, me, Kind::PendingIn).await?;
    tx.commit().await?;
    announce_add(state, target, me).await?;
    let rel = announce_add(state, me, target).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, rel))
}

/// Blockieren: Freundschaft bzw. Anfragen entfallen, der andere erfährt nur, dass die Beziehung weg ist.
async fn block(state: &AppState, me: Uuid, target: Uuid) -> ApiResult<Relationship> {
    if me == target {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(target)
        .fetch_one(&state.db)
        .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    let mut tx = state.db.begin().await?;
    lock_pair(&mut tx, me, target).await?;
    let mine = kind_of(&mut *tx, me, target).await?;
    let theirs = kind_of(&mut *tx, target, me).await?;
    if mine != Some(Kind::Blocked) {
        put_row(&mut tx, me, target, Kind::Blocked).await?;
    }
    let their_removed = theirs.is_some_and(|k| k != Kind::Blocked);
    if their_removed {
        delete_row(&mut tx, target, me).await?;
    }
    tx.commit().await?;
    // Laufender 1:1-Anruf endet, Klingeln zwischen den beiden hört auf.
    calls::blocked(state, me, target).await?;
    if mine == Some(Kind::Friend) {
        friendship_ended(state, me, target);
    }
    if their_removed {
        announce_remove(state, target, me);
    }
    announce_add(state, me, target).await?.ok_or_else(ApiError::not_found)
}

#[derive(Deserialize)]
pub struct RequestBody {
    username: String,
}

/// `POST /relationships` – Freundschaftsanfrage per Benutzername (Groß-/Kleinschreibung egal).
/// Unbekannte Nutzer sind von solchen, die keine Anfragen annehmen, nicht zu unterscheiden.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<RequestBody>,
) -> ApiResult<(StatusCode, Json<Relationship>)> {
    state.rate_limit.relationship_change(user.user_id)?;
    let username = body.username.trim();
    if username.is_empty() || username.chars().count() > 64 {
        return Err(ApiError::BadRequest("invalid_username"));
    }
    // Gesperrte Konten sind wie nicht vorhanden (auch für `PUT`, siehe `accepts_requests`).
    let target: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE lower(username) = lower($1) AND disabled_at IS NULL")
            .bind(username)
            .fetch_optional(&state.db)
            .await?;
    let Some(target) = target else {
        return Err(ApiError::Forbidden("friend_request_not_allowed"));
    };
    let (status, rel) = request(&state, user.user_id, target).await?;
    Ok((status, Json(rel)))
}

/// `GET /relationships` – eigene Freunde, Anfragen und Blockierte.
pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<Relationship>>> {
    Ok(Json(load(&state.db, user.user_id, None).await?))
}

#[derive(Deserialize)]
pub struct PutBody {
    /// `friend` (Standard): Anfrage senden bzw. annehmen; `blocked`: blockieren.
    #[serde(default = "default_put_kind", rename = "type")]
    kind: Kind,
}

fn default_put_kind() -> Kind {
    Kind::Friend
}

/// `PUT /relationships/{user_id}` – Anfrage senden/annehmen (`{ type: 'friend' }`) oder blockieren
/// (`{ type: 'blocked' }`).
pub async fn put(
    State(state): State<AppState>,
    user: AuthUser,
    Path(target): Path<Uuid>,
    body: Option<Json<PutBody>>,
) -> ApiResult<(StatusCode, Json<Relationship>)> {
    state.rate_limit.relationship_change(user.user_id)?;
    let kind = body.map_or(Kind::Friend, |b| b.kind);
    match kind {
        Kind::Friend => {
            let (status, rel) = request(&state, user.user_id, target).await?;
            Ok((status, Json(rel)))
        }
        Kind::Blocked => Ok((StatusCode::OK, Json(block(&state, user.user_id, target).await?))),
        Kind::PendingIn | Kind::PendingOut => Err(ApiError::BadRequest("invalid_request")),
    }
}

/// `DELETE /relationships/{user_id}` – Freund entfernen, Anfrage ablehnen/zurückziehen oder Blockade
/// aufheben. Ohne Beziehung passiert nichts (idempotent).
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(target): Path<Uuid>,
) -> ApiResult<StatusCode> {
    state.rate_limit.relationship_change(user.user_id)?;
    let me = user.user_id;
    let mut tx = state.db.begin().await?;
    lock_pair(&mut tx, me, target).await?;
    let Some(mine) = kind_of(&mut *tx, me, target).await? else {
        return Ok(StatusCode::NO_CONTENT);
    };
    delete_row(&mut tx, me, target).await?;
    // Freundschaften und Anfragen sind paarweise; eine Blockade gehört nur einem selbst.
    let pair = mine != Kind::Blocked;
    if pair {
        delete_row(&mut tx, target, me).await?;
    }
    tx.commit().await?;
    announce_remove(&state, me, target);
    if pair {
        announce_remove(&state, target, me);
    }
    if mine == Kind::Friend {
        friendship_ended(&state, me, target);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Wer darf einem Freundschaftsanfragen schicken?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequests {
    Everyone,
    MutualCommunities,
    Nobody,
}

impl FriendRequests {
    fn as_str(self) -> &'static str {
        match self {
            FriendRequests::Everyone => "everyone",
            FriendRequests::MutualCommunities => "mutual_communities",
            FriendRequests::Nobody => "nobody",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    friend_requests: FriendRequests,
}

/// `GET /relationships/settings`
pub async fn settings(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Settings>> {
    let value: String = sqlx::query_scalar("SELECT friend_requests FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    let friend_requests =
        serde_json::from_value(json!(value)).map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(Json(Settings { friend_requests }))
}

/// `PATCH /relationships/settings` – gilt für neue Anfragen; offene Anfragen bleiben bestehen.
pub async fn update_settings(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    sqlx::query("UPDATE users SET friend_requests = $2 WHERE id = $1")
        .bind(user.user_id)
        .bind(body.friend_requests.as_str())
        .execute(&state.db)
        .await?;
    Ok(Json(body))
}

/// Für die Konto-Löschung: Wer hat eine Beziehung zum Nutzer (Freunde, Anfragen, Blockierende)?
/// Die Zeilen löscht die Kaskade; danach `RELATIONSHIP_REMOVE` an diese Nutzer (`announce_deleted`).
pub async fn counterparts(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM relationships WHERE target_id = $1")
        .bind(user_id)
        .fetch_all(db)
        .await?)
}

/// Für `USER_UPDATE`: Wer darf das Profil des Nutzers über die Beziehung sehen (siehe
/// `models::can_see_user`)? Freunde und Nutzer, denen er eine Anfrage geschickt hat – nicht wer ihn
/// blockiert und nicht wer ihm selbst eine (unbeantwortete) Anfrage geschickt hat.
pub async fn profile_watchers(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM relationships WHERE target_id = $1 AND kind IN ('friend', 'pending_in')",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?)
}

pub fn announce_deleted(state: &AppState, user_id: Uuid, counterparts: &[Uuid]) {
    for &other in counterparts {
        announce_remove(state, other, user_id);
        state
            .events
            .control(Scope::User(other), Control::FriendRemoved { user_id: other, friend_id: user_id });
    }
}

/// Für den Datenexport: eigene Beziehungen.
pub async fn export(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<Relationship>> {
    load(db, user_id, None).await
}

/// Für den Datenexport: aktuelle Einstellung als Text.
pub async fn export_setting(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<String> {
    Ok(sqlx::query_scalar("SELECT friend_requests FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_serialize_snake_case() {
        assert_eq!(serde_json::to_value(Kind::PendingOut).unwrap(), json!("pending_out"));
        assert_eq!(serde_json::from_value::<Kind>(json!("blocked")).unwrap(), Kind::Blocked);
        assert_eq!(
            serde_json::from_value::<FriendRequests>(json!("mutual_communities")).unwrap().as_str(),
            "mutual_communities"
        );
    }
}
