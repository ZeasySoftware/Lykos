use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    config::LiveKitConfig,
    error::{ApiError, ApiResult},
    livekit::{self, WebhookEvent},
    models,
    permissions::{self, Permissions},
    routes::calls,
    state::AppState,
    voice::{self, VoiceState},
};

fn livekit_config(state: &AppState) -> ApiResult<&LiveKitConfig> {
    state.config.livekit.as_ref().ok_or(ApiError::ServiceUnavailable("voice_unavailable"))
}

#[derive(Serialize)]
pub struct JoinResponse {
    /// Öffentliche LiveKit-URL (wss://…).
    url: String,
    /// Kurzlebiges LiveKit-Access-Token für genau diesen Kanal.
    token: String,
    /// Ton-Einstellungen des Kanals für die eigene Mikrofonspur.
    audio: ChannelAudio,
}

/// Veröffentlichungsoptionen der Mikrofonspur, wie sie der Kanal vorgibt (in DM-Anrufen der Standard).
#[derive(Serialize)]
pub struct ChannelAudio {
    /// Ziel-Bitrate in kbit/s.
    bitrate: i32,
    stereo: bool,
    dtx: bool,
    red: bool,
}

impl ChannelAudio {
    /// Standard-Ton (wie die Spaltenvorgaben in Migration 0019): 64 kbit/s Mono, DTX und RED an.
    const DEFAULT: ChannelAudio = ChannelAudio { bitrate: 64, stereo: false, dtx: true, red: true };
}

/// Token zum Betreten eines Voice-Kanals. Rechte: `VIEW_CHANNEL` + `CONNECT`;
/// `SPEAK` erlaubt das Mikrofon, `STREAM` die Bildschirmübertragung, `VIDEO` die Kamera.
/// Ist der Kanal voll (`user_limit`), dürfen nur noch Nutzer mit `MANAGE_CHANNELS` hinein.
/// Im AFK-Kanal der Community hat niemand `SPEAK`, `STREAM` oder `VIDEO`.
///
/// DM-Kanäle: Token für den Anruf der DM (alle Teilnehmer dürfen sprechen, streamen, Kamera; Standard-Ton,
/// kein Nutzerlimit; bei Blockade in einer 1:1-DM `call_not_allowed`). Klingelt es bei einem, nimmt er damit
/// an; ist der Raum leer und klingelt es nicht, startet er einen Anruf (siehe `routes::calls`).
pub async fn join(
    State(state): State<AppState>,
    user: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<Json<JoinResponse>> {
    let cfg = livekit_config(&state)?;
    if let Some(channel) = models::channel_by_id(&state.db, channel_id).await?
        && models::is_dm_kind(&channel.kind)
    {
        return Ok(Json(join_dm(&state, cfg, user.user_id, &channel).await?));
    }
    let (_, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    let channel = models::channel_by_id(&state.db, channel_id).await?.ok_or_else(ApiError::not_found)?;
    if channel.kind != "voice" {
        return Err(ApiError::BadRequest("not_a_voice_channel"));
    }
    permissions::require(perms, Permissions::CONNECT)?;
    // Im AFK-Kanal: nur zuhören (das Token erlaubt kein Mikrofon, keinen Stream, keine Kamera).
    let perms = voice::afk_restricted(perms, voice::is_afk_channel(&state, channel_id).await?);
    if let Some(limit) = channel.user_limit
        && !perms.contains(Permissions::MANAGE_CHANNELS)
    {
        // Wer schon drin ist (z. B. Neuverbindung), zählt nicht gegen sich selbst.
        let others: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice_states WHERE channel_id = $1 AND user_id <> $2")
                .bind(channel_id)
                .bind(user.user_id)
                .fetch_one(&state.db)
                .await?;
        if others >= i64::from(limit) {
            return Err(ApiError::Forbidden("channel_full"));
        }
    }

    let token =
        livekit::join_token(cfg, user.user_id, &token_name(&state, user.user_id).await?, channel_id, perms);
    let audio = ChannelAudio {
        bitrate: channel.audio_bitrate,
        stereo: channel.audio_stereo,
        dtx: channel.audio_dtx,
        red: channel.audio_red,
    };
    Ok(Json(JoinResponse { url: cfg.url.clone(), token, audio }))
}

/// Anzeigename für LiveKit.
async fn token_name(state: &AppState, user_id: Uuid) -> ApiResult<String> {
    let me = models::user_by_id(&state.db, user_id).await?.ok_or(ApiError::Unauthorized("unauthorized"))?;
    Ok(me.display_name.unwrap_or(me.username))
}

async fn join_dm(
    state: &AppState,
    cfg: &LiveKitConfig,
    user_id: Uuid,
    channel: &models::Channel,
) -> ApiResult<JoinResponse> {
    let perms = permissions::dm_call_permissions(&state.db, user_id, channel.id, &channel.kind).await?;
    if !perms.contains(Permissions::CONNECT) {
        return Err(ApiError::Forbidden("call_not_allowed"));
    }
    // Annehmen, sonst – im leeren Raum ohne Klingeln – einen Anruf starten (mit Rate-Limit fürs Anklingeln).
    if !calls::stop_ringing(state, channel.id, user_id).await?
        && calls::is_idle(&state.db, channel.id, user_id).await?
    {
        state.rate_limit.call_ring(user_id)?;
        calls::ring(state, channel.id, user_id).await?;
    }
    let token = livekit::join_token(cfg, user_id, &token_name(state, user_id).await?, channel.id, perms);
    Ok(JoinResponse { url: cfg.url.clone(), token, audio: ChannelAudio::DEFAULT })
}

#[derive(Deserialize)]
pub struct SelfStateBody {
    self_mute: bool,
    self_deaf: bool,
}

/// Eigenen Mute/Deaf-Status melden (nur Anzeige für andere).
pub async fn set_state(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<SelfStateBody>,
) -> ApiResult<StatusCode> {
    voice::set_self_state(&state, user.user_id, body.self_mute, body.self_deaf).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Aktuelle Voice-States der Community (nur sichtbare Kanäle).
pub async fn community_states(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<VoiceState>>> {
    Ok(Json(voice::visible_states(&state, user.user_id, community_id).await?))
}

/// Webhook-Empfänger für LiveKit. Der `Authorization`-Header ist ein mit dem API-Secret
/// signiertes JWT, dessen `sha256`-Claim den Body absichert.
pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let cfg = livekit_config(&state)?;
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized("unauthorized"))?;
    if let Err(e) = livekit::verify_webhook(cfg, auth, &body) {
        tracing::warn!(error = %e, "LiveKit-Webhook abgelehnt");
        return Err(ApiError::Unauthorized("unauthorized"));
    }
    let event: WebhookEvent =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest("invalid_request"))?;
    tracing::debug!(event = %event.event, "LiveKit-Webhook");
    voice::handle_webhook(&state, event).await?;
    Ok(StatusCode::OK)
}
