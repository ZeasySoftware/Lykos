//! Direktnachrichten: 1:1-DMs und Gruppen-DMs (bis `MAX_GROUP_SIZE` Personen).
//!
//! DMs sind Kanäle ohne Community (`kind` = `dm` | `group_dm`); Nachrichten, Anhänge, Lesestand und
//! Tipp-Anzeige laufen über die normalen Kanal-Endpunkte. Rechte dort: `permissions::DM_PERMISSIONS`.
//! Eine DM eröffnen kann man mit Freunden und mit Nutzern, die mindestens eine Community mit einem teilen
//! und DMs erlauben (`users.allow_dms`) – Schutz vor Spam und vor dem Ausforschen fremder Konten.
//! Blockiert einer den anderen (`relationships`), geht das nie.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult, is_unique_violation},
    events::{Control, Scope},
    models::{UserPublic, double_option},
    routes::calls,
    state::AppState,
    voice,
};

/// Höchstzahl an Teilnehmern einer Gruppen-DM (inkl. Ersteller).
pub const MAX_GROUP_SIZE: usize = 10;
const MAX_NAME_CHARS: usize = 100;

/// DM-Kanal aus Sicht eines Nutzers.
#[derive(Debug, Clone, Serialize)]
pub struct DmChannel {
    pub id: Uuid,
    /// `dm` (1:1) oder `group_dm`.
    pub kind: String,
    /// Nur Gruppen-DMs; `null` = kein Name gesetzt (Clients zeigen die Teilnehmer).
    pub name: Option<String>,
    /// Alle Teilnehmer inkl. des Nutzers selbst, nach Benutzername sortiert.
    pub recipients: Vec<UserPublic>,
    pub last_message_id: Option<Uuid>,
    /// Eigener Lesestand. In Gateway-Events immer `null` – Clients behalten ihren Stand.
    pub last_read_message_id: Option<Uuid>,
    /// Ungelesene Nachrichten, die einen erwähnen. In Gateway-Events immer `0`.
    pub mention_count: i64,
}

#[derive(sqlx::FromRow)]
struct DmRow {
    id: Uuid,
    kind: String,
    name: String,
    last_message_id: Option<Uuid>,
    last_read_message_id: Option<Uuid>,
    mention_count: i64,
}

#[derive(sqlx::FromRow)]
struct RecipientRow {
    channel_id: Uuid,
    id: Uuid,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
}

/// DM-Kanäle eines Nutzers (alle oder nur `only`), neueste Aktivität zuerst.
async fn load_dms(state: &AppState, user_id: Uuid, only: Option<Uuid>) -> ApiResult<Vec<DmChannel>> {
    let rows: Vec<DmRow> = sqlx::query_as(
        "SELECT c.id, c.kind, c.name,
                (SELECT m.id FROM messages m WHERE m.channel_id = c.id ORDER BY m.id DESC LIMIT 1)
                    AS last_message_id,
                rs.last_read_message_id,
                -- Über `message_mentions (user_id, message_id)` statt über die ganze Historie;
                -- ohne Lesestand gilt die Null-UUID als untere Grenze (bleibt Indexbedingung).
                (SELECT count(*) FROM message_mentions mm JOIN messages m ON m.id = mm.message_id
                 WHERE mm.user_id = r.user_id
                   AND mm.message_id > COALESCE(rs.last_read_message_id,
                                                '00000000-0000-0000-0000-000000000000')
                   AND m.channel_id = c.id
                   AND m.author_id IS DISTINCT FROM r.user_id) AS mention_count
         FROM dm_recipients r
         JOIN channels c ON c.id = r.channel_id
         LEFT JOIN read_states rs ON rs.channel_id = c.id AND rs.user_id = r.user_id
         WHERE r.user_id = $1 AND ($2::uuid IS NULL OR c.id = $2)",
    )
    .bind(user_id)
    .bind(only)
    .fetch_all(&state.db)
    .await?;

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let recipients: Vec<RecipientRow> = sqlx::query_as(
        "SELECT r.channel_id, u.id, u.username, u.display_name, u.avatar_id
         FROM dm_recipients r JOIN users u ON u.id = r.user_id
         WHERE r.channel_id = ANY($1) ORDER BY lower(u.username)",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let mut by_channel: HashMap<Uuid, Vec<UserPublic>> = HashMap::new();
    for r in recipients {
        by_channel.entry(r.channel_id).or_default().push(UserPublic::new(
            r.id,
            r.username,
            r.display_name,
            r.avatar_id,
        ));
    }

    let mut dms: Vec<DmChannel> = rows
        .into_iter()
        .map(|r| DmChannel {
            recipients: by_channel.remove(&r.id).unwrap_or_default(),
            name: Some(r.name).filter(|n| !n.is_empty()),
            id: r.id,
            kind: r.kind,
            last_message_id: r.last_message_id,
            last_read_message_id: r.last_read_message_id,
            mention_count: r.mention_count,
        })
        .collect();
    // Neueste Aktivität zuerst (UUIDv7: Nachricht bzw. Kanal-Erstellung).
    dms.sort_by_key(|d| std::cmp::Reverse(d.last_message_id.unwrap_or(d.id)));
    Ok(dms)
}

async fn load_dm(state: &AppState, user_id: Uuid, channel_id: Uuid) -> ApiResult<DmChannel> {
    load_dms(state, user_id, Some(channel_id)).await?.pop().ok_or_else(ApiError::not_found)
}

/// Teilnehmer eines DM-Kanals.
async fn recipient_ids(db: impl PgExecutor<'_>, channel_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM dm_recipients WHERE channel_id = $1")
        .bind(channel_id)
        .fetch_all(db)
        .await?)
}

/// `GET /dms` – eigene DM-Kanäle.
pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<DmChannel>>> {
    Ok(Json(load_dms(&state, user.user_id, None).await?))
}

/// `GET /dms/contacts` – Nutzer, denen man eine DM schreiben kann: Freunde und Mitglieder gemeinsamer
/// Communities, die DMs erlauben (ohne einen selbst und ohne Blockaden in beide Richtungen).
pub async fn contacts(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<UserPublic>>> {
    #[derive(sqlx::FromRow)]
    struct ContactRow {
        id: Uuid,
        username: String,
        display_name: Option<String>,
        avatar_id: Option<Uuid>,
    }
    let rows: Vec<ContactRow> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, u.avatar_id
         FROM users u
         WHERE u.id <> $1 AND (
                (u.allow_dms AND EXISTS (
                    SELECT 1 FROM members mine JOIN members other ON other.community_id = mine.community_id
                    WHERE mine.user_id = $1 AND other.user_id = u.id))
             OR EXISTS (SELECT 1 FROM relationships f
                        WHERE f.user_id = $1 AND f.target_id = u.id AND f.kind = 'friend'))
           AND NOT EXISTS (SELECT 1 FROM relationships b WHERE b.kind = 'blocked'
                           AND ((b.user_id = $1 AND b.target_id = u.id) OR (b.user_id = u.id AND b.target_id = $1)))
         ORDER BY lower(u.username)",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter().map(|r| UserPublic::new(r.id, r.username, r.display_name, r.avatar_id)).collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateBody {
    recipient_ids: Vec<Uuid>,
    /// Nur für Gruppen-DMs.
    name: Option<String>,
}

fn normalize_name(name: Option<String>) -> ApiResult<String> {
    let name = name.map(|n| n.trim().to_owned()).unwrap_or_default();
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    Ok(name)
}

/// Schlüssel einer 1:1-DM, unabhängig von der Reihenfolge.
fn dm_key(a: Uuid, b: Uuid) -> String {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    format!("{lo}:{hi}")
}

/// Dürfen alle `recipients` von `user_id` angeschrieben werden? Freunde immer, sonst nur bei gemeinsamer
/// Community und erlaubten DMs; bei einer Blockade (egal in welche Richtung) und an gesperrte Konten nie.
/// Unbekannte und gesperrte Nutzer, Nutzer ohne gemeinsame Community, mit abgeschalteten DMs und Blockaden
/// sind nicht unterscheidbar (→ `dm_not_allowed`).
async fn check_reachable(state: &AppState, user_id: Uuid, recipients: &[Uuid]) -> ApiResult<()> {
    let reachable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users u
         WHERE u.id = ANY($2) AND u.disabled_at IS NULL
           AND (
                (u.allow_dms AND EXISTS (
                    SELECT 1 FROM members mine JOIN members other ON other.community_id = mine.community_id
                    WHERE mine.user_id = $1 AND other.user_id = u.id))
             OR EXISTS (SELECT 1 FROM relationships f
                        WHERE f.user_id = $1 AND f.target_id = u.id AND f.kind = 'friend'))
           AND NOT EXISTS (SELECT 1 FROM relationships b WHERE b.kind = 'blocked'
                           AND ((b.user_id = $1 AND b.target_id = u.id) OR (b.user_id = u.id AND b.target_id = $1)))",
    )
    .bind(user_id)
    .bind(recipients)
    .fetch_one(&state.db)
    .await?;
    if reachable as usize == recipients.len() { Ok(()) } else { Err(ApiError::Forbidden("dm_not_allowed")) }
}

/// Neuer DM-Kanal: Teilnehmer abonnieren ihn und bekommen `DM_CREATE`.
async fn announce_new(state: &AppState, channel_id: Uuid, recipients: &[Uuid]) -> ApiResult<()> {
    for &uid in recipients {
        state.events.control(Scope::User(uid), Control::DmJoined { user_id: uid, channel_id });
    }
    let mut dm = load_dm(state, recipients[0], channel_id).await?;
    dm.last_read_message_id = None;
    dm.mention_count = 0;
    state.events.publish(Scope::Dm(channel_id), "DM_CREATE", &dm);
    Ok(())
}

/// `POST /dms` – DM eröffnen. Bei genau einem Empfänger wird eine bestehende 1:1-DM wiederverwendet
/// (200), sonst entsteht ein neuer Kanal (201).
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<DmChannel>)> {
    state.rate_limit.dm_open(user.user_id)?;
    let mut others = body.recipient_ids;
    others.sort_unstable();
    others.dedup();
    if others.is_empty() || others.contains(&user.user_id) {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    if others.len() + 1 > MAX_GROUP_SIZE {
        return Err(ApiError::BadRequest("too_many_recipients"));
    }
    let mut participants = others.clone();
    participants.push(user.user_id);

    if let [other] = others[..] {
        if body.name.as_deref().is_some_and(|n| !n.trim().is_empty()) {
            return Err(ApiError::BadRequest("invalid_request"));
        }
        let key = dm_key(user.user_id, other);
        let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM channels WHERE dm_key = $1")
            .bind(&key)
            .fetch_optional(&state.db)
            .await?;
        if let Some(id) = existing {
            return Ok((StatusCode::OK, Json(load_dm(&state, user.user_id, id).await?)));
        }
        check_reachable(&state, user.user_id, &others).await?;

        let id = Uuid::now_v7();
        let mut tx = state.db.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO channels (id, community_id, name, kind, dm_key) VALUES ($1, NULL, '', 'dm', $2)",
        )
        .bind(id)
        .bind(&key)
        .execute(&mut *tx)
        .await;
        match inserted {
            // Gleichzeitig von beiden Seiten eröffnet: die andere Anfrage hat gewonnen.
            Err(e) if is_unique_violation(&e) => {
                drop(tx);
                let id: Uuid = sqlx::query_scalar("SELECT id FROM channels WHERE dm_key = $1")
                    .bind(&key)
                    .fetch_one(&state.db)
                    .await?;
                return Ok((StatusCode::OK, Json(load_dm(&state, user.user_id, id).await?)));
            }
            other => other?,
        };
        add_recipients(&mut tx, id, &participants).await?;
        tx.commit().await?;
        announce_new(&state, id, &participants).await?;
        return Ok((StatusCode::CREATED, Json(load_dm(&state, user.user_id, id).await?)));
    }

    let name = normalize_name(body.name)?;
    check_reachable(&state, user.user_id, &others).await?;
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO channels (id, community_id, name, kind) VALUES ($1, NULL, $2, 'group_dm')")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    add_recipients(&mut tx, id, &participants).await?;
    tx.commit().await?;
    announce_new(&state, id, &participants).await?;
    Ok((StatusCode::CREATED, Json(load_dm(&state, user.user_id, id).await?)))
}

async fn add_recipients(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: Uuid,
    users: &[Uuid],
) -> ApiResult<()> {
    sqlx::query("INSERT INTO dm_recipients (channel_id, user_id) SELECT $1, unnest($2::uuid[])")
        .bind(channel_id)
        .bind(users)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Kanal-Art eines DM-Kanals, an dem der Nutzer teilnimmt; sonst `NotFound`.
async fn own_dm_kind(state: &AppState, user_id: Uuid, channel_id: Uuid) -> ApiResult<String> {
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT c.kind FROM channels c JOIN dm_recipients r ON r.channel_id = c.id AND r.user_id = $2
         WHERE c.id = $1",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    kind.ok_or_else(ApiError::not_found)
}

#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(default, deserialize_with = "double_option")]
    name: Option<Option<String>>,
}

/// `PATCH /dms/{id}` – Namen einer Gruppen-DM setzen (`null`/leer = entfernen). Jeder Teilnehmer darf das.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<DmChannel>> {
    if own_dm_kind(&state, user.user_id, channel_id).await? != "group_dm" {
        return Err(ApiError::BadRequest("not_a_group_dm"));
    }
    if let Some(name) = body.name {
        let name = normalize_name(name)?;
        sqlx::query("UPDATE channels SET name = $2 WHERE id = $1")
            .bind(channel_id)
            .bind(&name)
            .execute(&state.db)
            .await?;
    }
    let dm = load_dm(&state, user.user_id, channel_id).await?;
    let mut event = dm.clone();
    event.last_read_message_id = None;
    event.mention_count = 0;
    state.events.publish(Scope::Dm(channel_id), "DM_UPDATE", &event);
    Ok(Json(dm))
}

/// `DELETE /dms/{id}` – Gruppen-DM verlassen. Verlässt der letzte Teilnehmer die Gruppe, wird der
/// Kanal samt Nachrichten gelöscht (Dateien räumt der Hintergrund-Job ab). 1:1-DMs kann man nicht verlassen.
pub async fn leave(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if own_dm_kind(&state, user.user_id, channel_id).await? != "group_dm" {
        return Err(ApiError::BadRequest("cannot_leave_dm"));
    }
    // Aus dem Anruf der Gruppe (solange man noch Teilnehmer ist, sehen alle das Verlassen).
    voice::leave_dm_call(&state, channel_id, user.user_id).await?;
    calls::stop_ringing(&state, channel_id, user.user_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM dm_recipients WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    let remaining = recipient_ids(&mut *tx, channel_id).await?;
    if remaining.is_empty() {
        sqlx::query("DELETE FROM channels WHERE id = $1").bind(channel_id).execute(&mut *tx).await?;
    }
    // Lesestand gehört nicht mehr zu einem sichtbaren Kanal.
    sqlx::query("DELETE FROM read_states WHERE channel_id = $1 AND user_id = $2")
        .bind(channel_id)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    state.events.control(Scope::User(user.user_id), Control::DmLeft { user_id: user.user_id, channel_id });
    state.events.publish(Scope::User(user.user_id), "DM_DELETE", json!({ "id": channel_id }));
    if !remaining.is_empty() {
        announce_update(&state, channel_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DM_UPDATE` an alle (verbliebenen) Teilnehmer, z. B. nachdem jemand gegangen ist oder sein Konto
/// gelöscht hat. Ohne Teilnehmer passiert nichts.
pub async fn announce_update(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let Some(&someone) = recipient_ids(&state.db, channel_id).await?.first() else {
        return Ok(());
    };
    let mut dm = load_dm(state, someone, channel_id).await?;
    dm.last_read_message_id = None;
    dm.mention_count = 0;
    state.events.publish(Scope::Dm(channel_id), "DM_UPDATE", &dm);
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct DmSettings {
    /// Mitglieder gemeinsamer Communities dürfen eine DM eröffnen.
    allow_dms: bool,
}

/// `GET /dms/settings`
pub async fn settings(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<DmSettings>> {
    let allow_dms: bool = sqlx::query_scalar("SELECT allow_dms FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(DmSettings { allow_dms }))
}

/// `PATCH /dms/settings` – gilt nur für neue DMs; bestehende Unterhaltungen bleiben.
pub async fn update_settings(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<DmSettings>,
) -> ApiResult<Json<DmSettings>> {
    sqlx::query("UPDATE users SET allow_dms = $2 WHERE id = $1")
        .bind(user.user_id)
        .bind(body.allow_dms)
        .execute(&state.db)
        .await?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_key_is_symmetric() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert_eq!(dm_key(a, b), dm_key(b, a));
        assert_ne!(dm_key(a, b), dm_key(a, a));
    }

    #[test]
    fn group_names() {
        assert_eq!(normalize_name(None).unwrap(), "");
        assert_eq!(normalize_name(Some("  Spieleabend ".into())).unwrap(), "Spieleabend");
        assert!(normalize_name(Some("x".repeat(101))).is_err());
    }
}
