//! Voice-Status: wer sitzt in welchem Voice-Kanal, gemutet/taub/streamt/Kamera an.
//!
//! Quelle der Wahrheit sind die LiveKit-Webhooks (`participant_joined`, `participant_left`,
//! `track_published`, ...). Mute/Deaf meldet der Client selbst (`POST /api/voice/state`).
//! Änderungen gehen als `VOICE_STATE_UPDATE`/`VOICE_STATE_DELETE` kanalgebunden über den Bus.
//!
//! DM-Anrufe (Migration 0021) laufen genauso: Der DM-Kanal ist der Raum, Voice-States haben dann keine
//! Community und gehen nur an die Teilnehmer der DM. Klingeln und Ablehnen regelt `routes::calls`.
//!
//! Rechteentzug (Kick, Verlassen, Overrides, Rollen) wirft Teilnehmer per RoomService aus LiveKit.
//! Ändern sich nur `SPEAK`/`STREAM`/`VIDEO`, bleiben sie im Raum: Ihre LiveKit-Rechte werden per
//! `UpdateParticipant` angeglichen und Spuren entzogener Quellen per `MutePublishedTrack`
//! stummgeschaltet. Diese Aufrufe laufen im Hintergrund und sind „best effort“: Ist LiveKit nicht
//! erreichbar, wird nur geloggt.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    events,
    livekit::{self, ParticipantInfo, WebhookEvent},
    models,
    permissions::{self, Permissions},
    routes::calls,
    state::AppState,
};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct VoiceState {
    /// `None` bei Anrufen in DMs.
    pub community_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub user_id: Uuid,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub streaming: bool,
    pub video: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

macro_rules! voice_columns {
    () => {
        "community_id, channel_id, user_id, self_mute, self_deaf, streaming, video, joined_at"
    };
}

/// Ohne diese Rechte fliegt man aus einem Voice-Kanal.
const REQUIRED: Permissions = Permissions(Permissions::VIEW_CHANNEL.0 | Permissions::CONNECT.0);

fn publish_update(state: &AppState, vs: &VoiceState) {
    state.events.publish(events::channel(vs.community_id, vs.channel_id), "VOICE_STATE_UPDATE", vs);
}

fn publish_delete(state: &AppState, community_id: Option<Uuid>, channel_id: Uuid, user_id: Uuid) {
    state.events.publish(
        events::channel(community_id, channel_id),
        "VOICE_STATE_DELETE",
        json!({ "community_id": community_id, "channel_id": channel_id, "user_id": user_id }),
    );
}

/// Voice-States einer Community in den Kanälen, die `user_id` sehen darf.
pub async fn visible_states(
    state: &AppState,
    user_id: Uuid,
    community_id: Uuid,
) -> ApiResult<Vec<VoiceState>> {
    let ctx = permissions::member_context(&state.db, user_id, community_id).await?;
    let overrides = permissions::community_overrides(&state.db, community_id).await?;
    let states: Vec<VoiceState> = sqlx::query_as(concat!(
        "SELECT ",
        voice_columns!(),
        " FROM voice_states WHERE community_id = $1 ORDER BY joined_at, user_id"
    ))
    .bind(community_id)
    .fetch_all(&state.db)
    .await?;
    Ok(states
        .into_iter()
        .filter(|vs| {
            ctx.in_channel(overrides.get(&vs.channel_id).map_or(&[], Vec::as_slice))
                .contains(Permissions::VIEW_CHANNEL)
        })
        .collect())
}

/// Mute/Deaf des Nutzers setzen. Ohne aktiven Voice-State wirkungslos.
pub async fn set_self_state(
    state: &AppState,
    user_id: Uuid,
    self_mute: bool,
    self_deaf: bool,
) -> ApiResult<()> {
    let vs: Option<VoiceState> = sqlx::query_as(concat!(
        "UPDATE voice_states SET self_mute = $2, self_deaf = $3
         WHERE user_id = $1 AND (self_mute <> $2 OR self_deaf <> $3)
         RETURNING ",
        voice_columns!()
    ))
    .bind(user_id)
    .bind(self_mute)
    .bind(self_deaf)
    .fetch_optional(&state.db)
    .await?;
    if let Some(vs) = vs {
        publish_update(state, &vs);
    }
    Ok(())
}

// --- Webhooks ---------------------------------------------------------------

/// Verarbeitet ein (bereits verifiziertes) LiveKit-Webhook-Event.
/// Räume heißen wie die Kanal-ID, Identities wie die Nutzer-ID; alles andere wird ignoriert.
pub async fn handle_webhook(state: &AppState, ev: WebhookEvent) -> ApiResult<()> {
    let Some(room) = ev.room.as_ref().and_then(|r| r.name.parse::<Uuid>().ok()) else {
        return Ok(());
    };
    if ev.event == "room_finished" {
        return room_finished(state, room).await;
    }
    let Some(participant) = ev.participant else {
        return Ok(());
    };
    let Ok(user_id) = participant.identity.parse::<Uuid>() else {
        return Ok(());
    };
    let sid = participant.sid.as_str();

    match ev.event.as_str() {
        "participant_joined" => joined(state, room, user_id, &participant).await,
        "participant_left" | "participant_connection_aborted" => left(state, room, user_id, sid).await,
        "track_published" | "track_unpublished" => {
            let published = ev.event == "track_published";
            match ev.track.as_ref() {
                Some(t) if t.is_screen_share() => set_streaming(state, room, user_id, sid, published).await,
                Some(t) if t.is_camera() => set_video(state, room, user_id, sid, published).await,
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

async fn joined(
    state: &AppState,
    channel_id: Uuid,
    user_id: Uuid,
    participant: &ParticipantInfo,
) -> ApiResult<()> {
    // Wer (inzwischen) nicht mehr darf, wird direkt wieder entfernt – Tokens gelten ja noch kurz.
    let channel = models::channel_by_id(&state.db, channel_id).await?.filter(|c| is_call_kind(&c.kind));
    let perms = match &channel {
        Some(c) => match call_permissions(state, c, user_id).await {
            Ok(perms) => perms,
            Err(ApiError::NotFound(_)) => Permissions::NONE,
            Err(e) => return Err(e),
        },
        None => Permissions::NONE,
    };
    let Some(channel) = channel.filter(|_| perms.contains(REQUIRED)) else {
        spawn_remove(state, channel_id, user_id);
        return Ok(());
    };

    // Spuren entzogener Quellen zählen nicht (sie werden unten stummgeschaltet).
    let allowed = livekit::allowed_sources(perms);
    let sid = participant.sid.as_str();
    let streaming = allowed.contains(&livekit::SOURCE_SCREEN_SHARE)
        && participant.tracks.iter().any(|t| t.is_screen_share());
    let video = allowed.contains(&livekit::SOURCE_CAMERA) && participant.tracks.iter().any(|t| t.is_camera());

    let mut tx = state.db.begin().await?;
    // Nutzerlimit auch beim tatsächlichen Beitritt: Join-Tokens gelten eine Weile, vorab geholte Tokens
    // dürfen den Kanal nicht überfüllen. (Statt `max_participants` am LiveKit-Raum – den legt LiveKit beim
    // ersten Beitritt selbst an, ein späteres Limit ließe sich dort nicht mehr setzen, und Nutzer mit
    // `MANAGE_CHANNELS` dürfen über das Limit hinaus.) Die Zeilensperre serialisiert gleichzeitige Beitritte.
    if let Some(limit) = channel.user_limit
        && !perms.contains(Permissions::MANAGE_CHANNELS)
    {
        sqlx::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        let others: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice_states WHERE channel_id = $1 AND user_id <> $2")
                .bind(channel_id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        if others >= i64::from(limit) {
            drop(tx);
            spawn_remove(state, channel_id, user_id);
            return Ok(());
        }
    }
    let previous: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT channel_id, community_id FROM voice_states WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let vs: VoiceState = sqlx::query_as(concat!(
        "INSERT INTO voice_states (user_id, channel_id, community_id, participant_sid, streaming, video)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (user_id) DO UPDATE SET
            channel_id = EXCLUDED.channel_id,
            community_id = EXCLUDED.community_id,
            participant_sid = EXCLUDED.participant_sid,
            streaming = EXCLUDED.streaming,
            video = EXCLUDED.video,
            joined_at = CASE WHEN voice_states.channel_id = EXCLUDED.channel_id
                             THEN voice_states.joined_at ELSE now() END
         RETURNING ",
        voice_columns!()
    ))
    .bind(user_id)
    .bind(channel_id)
    .bind(channel.community_id)
    .bind(sid)
    .bind(streaming)
    .bind(video)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    if let Some((prev_channel, prev_community)) = previous
        && prev_channel != channel_id
    {
        publish_delete(state, prev_community, prev_channel, user_id);
        if prev_community.is_none() {
            calls::room_left(state, prev_channel).await?;
        }
    }
    publish_update(state, &vs);
    // Angenommen: Bei wem es klingelte, der ist jetzt drin.
    if vs.community_id.is_none() {
        calls::stop_ringing(state, channel_id, user_id).await?;
    }

    // Token mit älteren (weiteren) Rechten: LiveKit-Rechte gleich beim Beitritt angleichen.
    if participant.needs_permission_sync(perms) {
        spawn_sync(state, channel_id, user_id, perms, Some(participant.clone()));
    }
    Ok(())
}

async fn left(state: &AppState, channel_id: Uuid, user_id: Uuid, sid: &str) -> ApiResult<()> {
    // Nur die eigene Verbindung: Ein verspätetes `left` einer alten Sitzung lässt die neue stehen.
    let community: Option<Option<Uuid>> = sqlx::query_scalar(
        "DELETE FROM voice_states
         WHERE user_id = $1 AND channel_id = $2 AND ($3 = '' OR participant_sid = $3)
         RETURNING community_id",
    )
    .bind(user_id)
    .bind(channel_id)
    .bind(sid)
    .fetch_optional(&state.db)
    .await?;
    if let Some(community_id) = community {
        publish_delete(state, community_id, channel_id, user_id);
        if community_id.is_none() {
            calls::room_left(state, channel_id).await?;
        }
    }
    Ok(())
}

async fn set_streaming(
    state: &AppState,
    channel_id: Uuid,
    user_id: Uuid,
    sid: &str,
    streaming: bool,
) -> ApiResult<()> {
    let vs: Option<VoiceState> = sqlx::query_as(concat!(
        "UPDATE voice_states SET streaming = $4
         WHERE user_id = $1 AND channel_id = $2 AND ($3 = '' OR participant_sid = $3) AND streaming <> $4
         RETURNING ",
        voice_columns!()
    ))
    .bind(user_id)
    .bind(channel_id)
    .bind(sid)
    .bind(streaming)
    .fetch_optional(&state.db)
    .await?;
    if let Some(vs) = vs {
        publish_update(state, &vs);
    }
    Ok(())
}

async fn set_video(
    state: &AppState,
    channel_id: Uuid,
    user_id: Uuid,
    sid: &str,
    video: bool,
) -> ApiResult<()> {
    let vs: Option<VoiceState> = sqlx::query_as(concat!(
        "UPDATE voice_states SET video = $4
         WHERE user_id = $1 AND channel_id = $2 AND ($3 = '' OR participant_sid = $3) AND video <> $4
         RETURNING ",
        voice_columns!()
    ))
    .bind(user_id)
    .bind(channel_id)
    .bind(sid)
    .bind(video)
    .fetch_optional(&state.db)
    .await?;
    if let Some(vs) = vs {
        publish_update(state, &vs);
    }
    Ok(())
}

async fn room_finished(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let removed: Vec<(Option<Uuid>, Uuid)> =
        sqlx::query_as("DELETE FROM voice_states WHERE channel_id = $1 RETURNING community_id, user_id")
            .bind(channel_id)
            .fetch_all(&state.db)
            .await?;
    let dm = removed.first().is_some_and(|(community_id, _)| community_id.is_none());
    for (community_id, user_id) in removed {
        publish_delete(state, community_id, channel_id, user_id);
    }
    if dm {
        calls::room_left(state, channel_id).await?;
    }
    Ok(())
}

// --- Rechteentzug -----------------------------------------------------------

/// Teilnehmer im Hintergrund aus dem LiveKit-Raum werfen.
fn spawn_remove(state: &AppState, channel_id: Uuid, user_id: Uuid) {
    let config = state.config.clone();
    if config.livekit.is_none() {
        return;
    }
    tokio::spawn(async move {
        let Some(cfg) = &config.livekit else { return };
        if let Err(e) = livekit::remove_participant(cfg, channel_id, user_id).await {
            tracing::warn!(error = %e, %channel_id, %user_id, "Teilnehmer konnte nicht entfernt werden");
        }
    });
}

/// LiveKit-Rechte eines verbundenen Teilnehmers im Hintergrund an `perms` angleichen und
/// stummgeschaltete Bildschirm-/Kameraspuren im Voice-State austragen.
fn spawn_sync(
    state: &AppState,
    channel_id: Uuid,
    user_id: Uuid,
    perms: Permissions,
    known: Option<ParticipantInfo>,
) {
    if state.config.livekit.is_none() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let Some(cfg) = &state.config.livekit else { return };
        match livekit::sync_permissions(cfg, channel_id, user_id, perms, known).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::warn!(error = %e, %channel_id, %user_id, "Voice-Rechte konnten nicht angeglichen werden");
                return;
            }
        }
        let allowed = livekit::allowed_sources(perms);
        let result = async {
            if !allowed.contains(&livekit::SOURCE_SCREEN_SHARE) {
                set_streaming(&state, channel_id, user_id, "", false).await?;
            }
            if !allowed.contains(&livekit::SOURCE_CAMERA) {
                set_video(&state, channel_id, user_id, "", false).await?;
            }
            ApiResult::Ok(())
        };
        if let Err(e) = result.await {
            tracing::warn!(error = %e, %channel_id, %user_id, "Voice-State nach Rechteentzug nicht aktualisiert");
        }
    });
}

/// Voice-State entfernen, Event senden und den Teilnehmer aus LiveKit werfen.
async fn evict(state: &AppState, user_id: Uuid, channel_id: Uuid) -> ApiResult<()> {
    let community: Option<Option<Uuid>> = sqlx::query_scalar(
        "DELETE FROM voice_states WHERE user_id = $1 AND channel_id = $2 RETURNING community_id",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(community_id) = community {
        publish_delete(state, community_id, channel_id, user_id);
        if community_id.is_none() {
            calls::room_left(state, channel_id).await?;
        }
    }
    spawn_remove(state, channel_id, user_id);
    Ok(())
}

/// Nutzer aus seinem Sprachkanal bzw. Anruf werfen, wo auch immer er ist (Sperre, Konto-Löschung).
pub async fn disconnect_user(state: &AppState, user_id: Uuid) -> ApiResult<()> {
    let channel: Option<Uuid> = sqlx::query_scalar("SELECT channel_id FROM voice_states WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;
    match channel {
        Some(channel_id) => evict(state, user_id, channel_id).await,
        None => Ok(()),
    }
}

/// Nutzer aus dem Anruf eines DM-Kanals werfen (Gruppe verlassen), falls er drin ist.
pub async fn leave_dm_call(state: &AppState, channel_id: Uuid, user_id: Uuid) -> ApiResult<()> {
    let inside: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM voice_states WHERE user_id = $1 AND channel_id = $2)",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_one(&state.db)
    .await?;
    if inside { evict(state, user_id, channel_id).await } else { Ok(()) }
}

/// Anruf eines DM-Kanals beenden: alle hinauswerfen, Klingeln beenden und den LiveKit-Raum schließen
/// (Blockade in einer 1:1-DM).
pub async fn end_dm_call(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM voice_states WHERE channel_id = $1")
        .bind(channel_id)
        .fetch_all(&state.db)
        .await?;
    let active = !users.is_empty();
    for user_id in users {
        evict(state, user_id, channel_id).await?;
    }
    calls::end_ringing(state, channel_id, false).await?;
    if active {
        close_rooms(state, vec![channel_id]);
    }
    Ok(())
}

/// Kann der Kanal ein LiveKit-Raum sein? Sprachkanäle und DMs (Anrufe).
pub fn is_call_kind(kind: &str) -> bool {
    kind == "voice" || models::is_dm_kind(kind)
}

/// Rechte des Nutzers im Raum des Kanals: Kanalrechte bei Sprachkanälen, `DM_CALL_PERMISSIONS` in DMs
/// (keine bei Blockade in einer 1:1-DM). `NotFound`, wenn er den Kanal nicht sehen darf.
pub async fn call_permissions(
    state: &AppState,
    channel: &models::Channel,
    user_id: Uuid,
) -> ApiResult<Permissions> {
    if models::is_dm_kind(&channel.kind) {
        return permissions::dm_call_permissions(&state.db, user_id, channel.id, &channel.kind).await;
    }
    Ok(permissions::channel_permissions(&state.db, user_id, channel.id).await?.1)
}

/// Mitglied hat die Community verlassen oder wurde entfernt: aus ihrem Voice werfen.
pub async fn member_removed(state: &AppState, community_id: Uuid, user_id: Uuid) -> ApiResult<()> {
    let channel: Option<Uuid> =
        sqlx::query_scalar("SELECT channel_id FROM voice_states WHERE user_id = $1 AND community_id = $2")
            .bind(user_id)
            .bind(community_id)
            .fetch_optional(&state.db)
            .await?;
    match channel {
        Some(channel_id) => evict(state, user_id, channel_id).await,
        None => Ok(()),
    }
}

/// Rechte in der Community haben sich geändert (Overrides, Rollen): alle Voice-Teilnehmer
/// neu prüfen und entfernen, wer `VIEW_CHANNEL`/`CONNECT` verloren hat. Läuft im Hintergrund.
pub fn recheck_community(state: &AppState, community_id: Uuid) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = recheck(&state, community_id).await {
            tracing::warn!(error = %e, %community_id, "Voice-Rechteprüfung fehlgeschlagen");
        }
    });
}

async fn recheck(state: &AppState, community_id: Uuid) -> ApiResult<()> {
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT user_id, channel_id FROM voice_states WHERE community_id = $1")
            .bind(community_id)
            .fetch_all(&state.db)
            .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let overrides = permissions::community_overrides(&state.db, community_id).await?;
    for (user_id, channel_id) in rows {
        let perms = match permissions::member_context(&state.db, user_id, community_id).await {
            Ok(ctx) => ctx.in_channel(overrides.get(&channel_id).map_or(&[], Vec::as_slice)),
            Err(ApiError::NotFound(_)) => Permissions::NONE,
            Err(e) => return Err(e),
        };
        if perms.contains(REQUIRED) {
            // Darf bleiben: SPEAK/STREAM/VIDEO in LiveKit angleichen (fragt den aktuellen Stand ab).
            spawn_sync(state, channel_id, user_id, perms, None);
        } else {
            evict(state, user_id, channel_id).await?;
        }
    }
    Ok(())
}

/// Kanäle der Community mit aktiven Teilnehmern (vor dem Löschen der Community abfragen).
pub async fn active_rooms(state: &AppState, community_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT DISTINCT channel_id FROM voice_states WHERE community_id = $1")
        .bind(community_id)
        .fetch_all(&state.db)
        .await?)
}

/// LiveKit-Räume schließen (Kanal oder Community gelöscht). Läuft im Hintergrund.
pub fn close_rooms(state: &AppState, rooms: Vec<Uuid>) {
    let config = state.config.clone();
    if config.livekit.is_none() || rooms.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let Some(cfg) = &config.livekit else { return };
        for room in rooms {
            if let Err(e) = livekit::delete_room(cfg, room).await {
                tracing::warn!(error = %e, %room, "LiveKit-Raum konnte nicht geschlossen werden");
            }
        }
    });
}

// --- Serverstart ------------------------------------------------------------

/// Gleicht `voice_states` beim Start mit LiveKit ab: verwaiste Einträge (Webhooks verpasst,
/// während der Server weg war) werden gelöscht, unbekannte Teilnehmer nachgetragen.
/// Ohne LiveKit-Konfiguration wird alles gelöscht; ist LiveKit nicht erreichbar, bleibt alles.
pub fn spawn_startup_sync(state: AppState) {
    tokio::spawn(async move {
        // LiveKit startet oft gleichzeitig mit dem Server (Compose, Reboot) – daher mehrere Versuche.
        let mut delay = std::time::Duration::from_secs(2);
        for attempt in 1..=6 {
            match startup_sync(&state).await {
                Ok(()) => return,
                Err(e) if attempt < 6 => {
                    tracing::debug!(error = %e, attempt, "Voice-Abgleich beim Start: neuer Versuch");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => tracing::warn!(error = %e, "Voice-Abgleich beim Start fehlgeschlagen"),
            }
        }
    });
}

pub async fn startup_sync(state: &AppState) -> anyhow::Result<()> {
    // Klingeln überlebt keinen Neustart (Clients holen den Zustand mit READY neu).
    sqlx::query("DELETE FROM call_rings").execute(&state.db).await?;
    let Some(cfg) = &state.config.livekit else {
        sqlx::query("DELETE FROM voice_states").execute(&state.db).await?;
        return Ok(());
    };

    let mut live: HashSet<(Uuid, Uuid)> = HashSet::new();
    for room in livekit::list_rooms(cfg).await? {
        let Ok(channel_id) = room.parse::<Uuid>() else { continue };
        for p in livekit::list_participants(cfg, &room).await? {
            let Ok(user_id) = p.identity.parse::<Uuid>() else { continue };
            live.insert((user_id, channel_id));
            joined(state, channel_id, user_id, &p).await.map_err(|e| anyhow::anyhow!(e))?;
        }
    }

    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT user_id, channel_id FROM voice_states").fetch_all(&state.db).await?;
    let mut removed = 0;
    for (user_id, channel_id) in rows {
        if !live.contains(&(user_id, channel_id)) {
            left(state, channel_id, user_id, "").await.map_err(|e| anyhow::anyhow!(e))?;
            removed += 1;
        }
    }
    tracing::info!(active = live.len(), removed, "Voice-Status mit LiveKit abgeglichen");
    Ok(())
}
