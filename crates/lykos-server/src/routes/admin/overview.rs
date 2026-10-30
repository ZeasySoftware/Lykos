//! Dashboard: Zählwerte und Betriebsdaten der Instanz (keine Inhalte, keine personenbezogenen Details).

use axum::{Json, extract::State};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{self, InstanceAdmin},
    config::StorageBackend,
    error::ApiResult,
    state::AppState,
};

#[derive(Serialize, FromRow)]
pub struct UserStats {
    total: i64,
    new_7d: i64,
    disabled: i64,
    admins: i64,
}

#[derive(Serialize, FromRow)]
pub struct CommunityStorage {
    id: Uuid,
    name: String,
    bytes: i64,
}

#[derive(Serialize)]
pub struct StorageStats {
    /// Summe aller Anhänge in Bytes.
    total_bytes: i64,
    attachment_count: i64,
    /// `local` oder `s3`.
    backend: &'static str,
    /// Größte Communities nach Anhang-Speicher (max. 5).
    top_communities: Vec<CommunityStorage>,
}

#[derive(Serialize, FromRow)]
pub struct VoiceChannelStats {
    community_id: Uuid,
    community_name: String,
    channel_id: Uuid,
    channel_name: String,
    participants: i64,
}

#[derive(Serialize)]
pub struct VoiceStats {
    /// Voice über LiveKit konfiguriert?
    enabled: bool,
    participants: i64,
    channels: Vec<VoiceChannelStats>,
}

#[derive(Serialize)]
pub struct Overview {
    users: UserStats,
    communities: i64,
    /// Anzahl Nachrichten in Communities und DMs (keine Inhalte).
    messages: i64,
    storage: StorageStats,
    /// Für andere sichtbar online (Unsichtbare zählen nicht).
    online: usize,
    voice: VoiceStats,
    version: &'static str,
    database_bytes: i64,
    uptime_seconds: u64,
}

/// `GET /admin/overview`
pub async fn overview(State(state): State<AppState>, _admin: InstanceAdmin) -> ApiResult<Json<Overview>> {
    let users: UserStats = sqlx::query_as(
        "SELECT count(*) AS total,
                count(*) FILTER (WHERE created_at > now() - interval '7 days') AS new_7d,
                count(*) FILTER (WHERE disabled_at IS NOT NULL) AS disabled,
                count(*) FILTER (WHERE is_instance_admin AND disabled_at IS NULL) AS admins
         FROM users",
    )
    .fetch_one(&state.db)
    .await?;
    let (communities, messages, database_bytes): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM communities), (SELECT count(*) FROM messages),
                pg_database_size(current_database())",
    )
    .fetch_one(&state.db)
    .await?;
    let (total_bytes, attachment_count): (i64, i64) =
        sqlx::query_as("SELECT COALESCE(sum(size_bytes), 0)::BIGINT, count(*) FROM attachments")
            .fetch_one(&state.db)
            .await?;
    let top_communities: Vec<CommunityStorage> = sqlx::query_as(
        "SELECT c.id, c.name, sum(a.size_bytes)::BIGINT AS bytes
         FROM attachments a JOIN communities c ON c.id = a.community_id
         GROUP BY c.id, c.name ORDER BY bytes DESC LIMIT 5",
    )
    .fetch_all(&state.db)
    .await?;
    let channels: Vec<VoiceChannelStats> = sqlx::query_as(
        "SELECT co.id AS community_id, co.name AS community_name, ch.id AS channel_id, ch.name AS channel_name,
                count(*) AS participants
         FROM voice_states v
         JOIN channels ch ON ch.id = v.channel_id
         JOIN communities co ON co.id = v.community_id
         GROUP BY co.id, co.name, ch.id, ch.name
         ORDER BY participants DESC, co.name, ch.name",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(Overview {
        users,
        communities,
        messages,
        storage: StorageStats {
            total_bytes,
            attachment_count,
            backend: match state.config.storage {
                StorageBackend::Local => "local",
                StorageBackend::S3(_) => "s3",
            },
            top_communities,
        },
        online: state.presence.online_count(),
        voice: VoiceStats {
            enabled: state.config.livekit.is_some(),
            participants: channels.iter().map(|c| c.participants).sum(),
            channels,
        },
        version: env!("CARGO_PKG_VERSION"),
        database_bytes,
        uptime_seconds: admin::uptime_seconds(),
    }))
}
