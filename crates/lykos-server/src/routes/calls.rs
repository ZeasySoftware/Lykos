//! Anrufe in Direktnachrichten (1:1 und Gruppen): Klingeln, Ablehnen und Anrufzustand.
//!
//! Ein DM-Anruf ist der LiveKit-Raum des DM-Kanals. Beitreten geht wie bei Sprachkanälen über
//! `POST /channels/{id}/voice/join` (siehe `routes::voice`), wer drin ist, steht in `voice_states` (ohne
//! Community, Events nur an die Teilnehmer der DM). Holt sich jemand ein Token, während der Raum leer ist und
//! es nicht schon klingelt, startet er damit einen Anruf: Bei den anderen Teilnehmern klingelt es
//! (`call_rings`) für `Config::call_ring_secs`. Das Klingeln endet für einen Nutzer, wenn er beitritt oder
//! ablehnt, und für alle mit Ablauf oder wenn der Raum wieder leer ist (Anrufer legt allein auf). Bei einer
//! Blockade klingelt es nicht; in einer 1:1-DM gibt es dann gar keinen Anruf.
//!
//! In einer 1:1-DM hinterlässt ein nicht angenommener Anruf die Systemnachricht `call_missed` (Autor = Anrufer).
//! Den Anrufzustand bekommen alle Teilnehmer als `CALL_UPDATE { channel_id, ringing, caller_id }`, laufende
//! Anrufe der eigenen DMs zusätzlich mit READY/RESYNC (`dm_calls`).

use std::{collections::BTreeMap, time::Duration};

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::messages;
use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::Scope,
    models, permissions,
    state::AppState,
    voice::VoiceState,
};

/// Anrufzustand eines DM-Kanals (`CALL_UPDATE`).
#[derive(Debug, Clone, Serialize)]
pub struct CallState {
    pub channel_id: Uuid,
    /// Bei wem es gerade klingelt.
    pub ringing: Vec<Uuid>,
    /// Wer angerufen hat (solange es klingelt), sonst `null`.
    pub caller_id: Option<Uuid>,
}

/// Laufender Anruf in einer DM für READY/RESYNC: Teilnehmer und Klingelzustand.
#[derive(Debug, Clone, Serialize)]
pub struct DmCall {
    pub channel_id: Uuid,
    pub voice_states: Vec<VoiceState>,
    pub ringing: Vec<Uuid>,
    pub caller_id: Option<Uuid>,
}

async fn load_state(db: impl PgExecutor<'_>, channel_id: Uuid) -> ApiResult<CallState> {
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT user_id, caller_id FROM call_rings WHERE channel_id = $1 AND expires_at > now()
         ORDER BY user_id",
    )
    .bind(channel_id)
    .fetch_all(db)
    .await?;
    Ok(CallState {
        channel_id,
        caller_id: rows.first().map(|r| r.1),
        ringing: rows.into_iter().map(|r| r.0).collect(),
    })
}

/// Aktuellen Klingelzustand an alle Teilnehmer der DM.
async fn announce(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let call = load_state(&state.db, channel_id).await?;
    state.events.publish(Scope::Dm(channel_id), "CALL_UPDATE", &call);
    Ok(())
}

/// Neu hinzugefügte Teilnehmer einer Gruppen-DM bekommen einen laufenden Anruf mit: die Voice-States als
/// `VOICE_STATE_UPDATE` und ggf. das Klingeln als `CALL_UPDATE` (READY/RESYNC kennen sie noch nicht). Der Anruf
/// selbst bleibt unberührt; es klingelt bei den Neuen nicht.
pub async fn sync_new_recipients(state: &AppState, channel_id: Uuid, users: &[Uuid]) -> ApiResult<()> {
    let states: Vec<VoiceState> = sqlx::query_as(
        "SELECT community_id, channel_id, user_id, self_mute, self_deaf, streaming, video, joined_at
         FROM voice_states WHERE channel_id = $1 ORDER BY joined_at, user_id",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?;
    let call = load_state(&state.db, channel_id).await?;
    for &uid in users {
        for vs in &states {
            state.events.publish(Scope::User(uid), "VOICE_STATE_UPDATE", vs);
        }
        if !call.ringing.is_empty() {
            state.events.publish(Scope::User(uid), "CALL_UPDATE", &call);
        }
    }
    Ok(())
}

/// Ist im Raum außer `user_id` niemand und klingelt es nirgends? Dann startet ein Beitritt einen Anruf.
pub async fn is_idle(db: &sqlx::PgPool, channel_id: Uuid, user_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM voice_states WHERE channel_id = $1 AND user_id <> $2)
            AND NOT EXISTS (SELECT 1 FROM call_rings WHERE channel_id = $1 AND expires_at > now())",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(db)
    .await?)
}

/// Bei allen anderen Teilnehmern klingeln lassen, die nicht schon im Anruf sind, deren Konto nicht gesperrt
/// ist und zwischen denen und dem Anrufer es keine Blockade gibt. Gibt zurück, ob es irgendwo klingelt.
pub async fn ring(state: &AppState, channel_id: Uuid, caller: Uuid) -> ApiResult<bool> {
    let rung: Vec<Uuid> = sqlx::query_scalar(
        "INSERT INTO call_rings (channel_id, user_id, caller_id, expires_at)
         SELECT $1, r.user_id, $2, now() + make_interval(secs => $3)
         FROM dm_recipients r JOIN users u ON u.id = r.user_id
         WHERE r.channel_id = $1 AND r.user_id <> $2 AND u.disabled_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM voice_states v WHERE v.user_id = r.user_id AND v.channel_id = $1)
           AND NOT EXISTS (SELECT 1 FROM relationships b WHERE b.kind = 'blocked'
                           AND ((b.user_id = $2 AND b.target_id = r.user_id)
                             OR (b.user_id = r.user_id AND b.target_id = $2)))
         ON CONFLICT (channel_id, user_id) DO UPDATE
            SET caller_id = EXCLUDED.caller_id, expires_at = EXCLUDED.expires_at
         RETURNING user_id",
    )
    .bind(channel_id)
    .bind(caller)
    .bind(state.config.call_ring_secs as f64)
    .fetch_all(&state.db)
    .await?;
    if rung.is_empty() {
        return Ok(false);
    }
    announce(state, channel_id).await?;
    spawn_expiry(state);
    Ok(true)
}

/// Klingeln für einen Nutzer beenden (angenommen, abgelehnt, Gruppe verlassen). Gibt zurück, ob es bei ihm
/// noch klingelte.
pub async fn stop_ringing(state: &AppState, channel_id: Uuid, user_id: Uuid) -> ApiResult<bool> {
    let active: Option<bool> = sqlx::query_scalar(
        "DELETE FROM call_rings WHERE channel_id = $1 AND user_id = $2 RETURNING expires_at > now()",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    let ringing = active == Some(true);
    if ringing {
        announce(state, channel_id).await?;
    }
    Ok(ringing)
}

/// Jemand hat den Raum eines DM-Kanals verlassen: Ist er jetzt leer, endet das Klingeln (der Anrufer hat
/// allein aufgelegt) – in 1:1-DMs als verpasster Anruf.
pub async fn room_left(state: &AppState, channel_id: Uuid) -> ApiResult<()> {
    let empty: bool =
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM voice_states WHERE channel_id = $1)")
            .bind(channel_id)
            .fetch_one(&state.db)
            .await?;
    if empty {
        end_ringing(state, channel_id, true).await?;
    }
    Ok(())
}

/// Klingeln im DM-Kanal für alle beenden. `missed`: in 1:1-DMs „Anruf verpasst“ hinterlassen.
pub async fn end_ringing(state: &AppState, channel_id: Uuid, missed: bool) -> ApiResult<()> {
    let rows: Vec<(Uuid, bool)> = sqlx::query_as(
        "DELETE FROM call_rings WHERE channel_id = $1 RETURNING caller_id, expires_at > now()",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?;
    // Abgelaufene Zeilen wurden schon gemeldet (oder sind von vor einem Neustart) – nur aktive zählen.
    let Some(caller) = rows.iter().find(|r| r.1).map(|r| r.0) else {
        return Ok(());
    };
    announce(state, channel_id).await?;
    if missed {
        post_missed(state, channel_id, caller).await?;
    }
    Ok(())
}

/// Systemnachricht „Anruf verpasst“ (nur in 1:1-DMs).
async fn post_missed(state: &AppState, channel_id: Uuid, caller: Uuid) -> ApiResult<()> {
    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, kind)
         SELECT $1, c.id, $3, '', 'call_missed' FROM channels c
         WHERE c.id = $2 AND c.kind = 'dm' AND EXISTS (SELECT 1 FROM users WHERE id = $3)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(caller)
    .execute(&state.db)
    .await?
    .rows_affected();
    if inserted > 0 {
        messages::broadcast(state, "MESSAGE_CREATE", id).await?;
    }
    Ok(())
}

/// Nach Ablauf der Klingeldauer abgelaufene Zeilen abräumen und melden (läuft im Hintergrund).
fn spawn_expiry(state: &AppState) {
    let state = state.clone();
    let delay = Duration::from_secs(state.config.call_ring_secs) + Duration::from_millis(250);
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if let Err(e) = expire(&state).await {
            tracing::warn!(error = %e, "Abgelaufenes Klingeln konnte nicht abgeräumt werden");
        }
    });
}

/// Abgelaufenes Klingeln aller DMs entfernen, `CALL_UPDATE` senden und in 1:1-DMs „Anruf verpasst“
/// hinterlassen.
pub async fn expire(state: &AppState) -> ApiResult<()> {
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("DELETE FROM call_rings WHERE expires_at <= now() RETURNING channel_id, caller_id")
            .fetch_all(&state.db)
            .await?;
    let by_channel: BTreeMap<Uuid, Uuid> = rows.into_iter().collect();
    for (channel_id, caller) in by_channel {
        announce(state, channel_id).await?;
        post_missed(state, channel_id, caller).await?;
    }
    Ok(())
}

/// Laufende Anrufe (Teilnehmer oder Klingeln) in den DMs des Nutzers – für READY/RESYNC.
pub async fn for_user(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Vec<DmCall>> {
    let states: Vec<VoiceState> = sqlx::query_as(
        "SELECT v.community_id, v.channel_id, v.user_id, v.self_mute, v.self_deaf, v.streaming, v.video,
                v.joined_at
         FROM voice_states v JOIN dm_recipients r ON r.channel_id = v.channel_id AND r.user_id = $1
         WHERE v.community_id IS NULL ORDER BY v.joined_at, v.user_id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let rings: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT c.channel_id, c.user_id, c.caller_id
         FROM call_rings c JOIN dm_recipients r ON r.channel_id = c.channel_id AND r.user_id = $1
         WHERE c.expires_at > now() ORDER BY c.user_id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let mut calls: BTreeMap<Uuid, DmCall> = BTreeMap::new();
    let entry = |calls: &mut BTreeMap<Uuid, DmCall>, channel_id: Uuid| {
        calls.entry(channel_id).or_insert_with(|| DmCall {
            channel_id,
            voice_states: Vec::new(),
            ringing: Vec::new(),
            caller_id: None,
        });
    };
    for vs in states {
        entry(&mut calls, vs.channel_id);
        calls.get_mut(&vs.channel_id).expect("eben angelegt").voice_states.push(vs);
    }
    for (channel_id, user, caller) in rings {
        entry(&mut calls, channel_id);
        let call = calls.get_mut(&channel_id).expect("eben angelegt");
        call.ringing.push(user);
        call.caller_id.get_or_insert(caller);
    }
    Ok(calls.into_values().collect())
}

/// Blockade zwischen `a` und `b`: Ihr 1:1-Anruf endet (beide fliegen raus), und in Gruppen-DMs klingelt es
/// zwischen den beiden nicht mehr.
pub async fn blocked(state: &AppState, a: Uuid, b: Uuid) -> ApiResult<()> {
    let dm: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM channels c
         WHERE c.kind = 'dm'
           AND EXISTS (SELECT 1 FROM dm_recipients r WHERE r.channel_id = c.id AND r.user_id = $1)
           AND EXISTS (SELECT 1 FROM dm_recipients r WHERE r.channel_id = c.id AND r.user_id = $2)",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(&state.db)
    .await?;
    if let Some(channel_id) = dm {
        crate::voice::end_dm_call(state, channel_id).await?;
    }
    let channels: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM call_rings
         WHERE (user_id = $1 AND caller_id = $2) OR (user_id = $2 AND caller_id = $1)
         RETURNING channel_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&state.db)
    .await?;
    announce_all(state, channels).await
}

/// Konto wird gelöscht: Klingeln bei ihm und von ihm beenden (vor dem Löschen aufrufen).
pub async fn user_removed(state: &AppState, user_id: Uuid) -> ApiResult<()> {
    let channels: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM call_rings WHERE user_id = $1 OR caller_id = $1 RETURNING channel_id",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    announce_all(state, channels).await
}

async fn announce_all(state: &AppState, mut channels: Vec<Uuid>) -> ApiResult<()> {
    channels.sort_unstable();
    channels.dedup();
    for channel_id in channels {
        announce(state, channel_id).await?;
    }
    Ok(())
}

/// DM-Kanal, an dem der Nutzer teilnimmt; sonst `NotFound`.
async fn own_dm(state: &AppState, user_id: Uuid, channel_id: Uuid) -> ApiResult<models::Channel> {
    let channel = models::channel_by_id(&state.db, channel_id)
        .await?
        .filter(|c| models::is_dm_kind(&c.kind))
        .ok_or_else(ApiError::not_found)?;
    if !permissions::is_dm_recipient(&state.db, user_id, channel_id).await? {
        return Err(ApiError::not_found());
    }
    Ok(channel)
}

/// `POST /channels/{id}/call/decline` – Anruf ablehnen: Das Klingeln endet für einen selbst. Wer selbst
/// angerufen hat und mit niemandem verbunden ist, bricht damit das Klingeln für alle ab (Auflegen, bevor
/// jemand angenommen hat). Sonst wirkungslos.
pub async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let me = user.user_id;
    own_dm(&state, me, channel_id).await?;
    if stop_ringing(&state, channel_id, me).await? {
        return Ok(StatusCode::NO_CONTENT);
    }
    let cancel: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM call_rings WHERE channel_id = $1 AND caller_id = $2 AND expires_at > now())
            AND NOT EXISTS (SELECT 1 FROM voice_states WHERE channel_id = $1 AND user_id <> $2)",
    )
    .bind(channel_id)
    .bind(me)
    .fetch_one(&state.db)
    .await?;
    if cancel {
        end_ringing(&state, channel_id, true).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /channels/{id}/call/ring` – während man im Anruf ist, bei allen anderen (erneut) klingeln lassen,
/// die noch nicht drin sind (z. B. nach Ablehnen oder Ablauf). Rate-Limit wie beim Anrufen.
pub async fn ring_again(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let me = user.user_id;
    let channel = own_dm(&state, me, channel_id).await?;
    let perms = permissions::dm_call_permissions(&state.db, me, channel_id, &channel.kind).await?;
    permissions::require(perms, permissions::Permissions::CONNECT)?;
    let inside: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM voice_states WHERE channel_id = $1 AND user_id = $2)",
    )
    .bind(channel_id)
    .bind(me)
    .fetch_one(&state.db)
    .await?;
    if !inside {
        return Err(ApiError::BadRequest("not_in_call"));
    }
    state.rate_limit.call_ring(me)?;
    ring(&state, channel_id, me).await?;
    Ok(StatusCode::NO_CONTENT)
}
