//! Speicher-Kontingente für Nachrichten-Anhänge.
//!
//! Belegt ist die Summe aller Anhänge (auch noch nicht gesendeter Uploads) – pro Nutzer
//! (`LYKOS_USER_QUOTA_MB`) und optional pro Community (`communities.upload_quota_mb`).
//! Andere Dateien (z. B. Avatare) zählen bewusst nicht mit.
//!
//! Der Upload prüft vorab den Restplatz (damit große Dateien früh abbrechen) und beim Eintragen
//! noch einmal unter einer Transaktions-Sperre je Nutzer und Community – parallele Uploads können
//! das Kontingent so nicht gemeinsam überschreiten.

use serde::Serialize;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::{
    attachments::{self, Attachment, NewAttachment},
    error::{ApiError, ApiResult},
    instance_settings,
    state::AppState,
};

const MB: u64 = 1024 * 1024;

/// Fehler bei überschrittenem Kontingent (413).
pub fn exceeded() -> ApiError {
    ApiError::PayloadTooLarge("quota_exceeded")
}

/// Belegung und Kontingent, wie sie die API ausliefert (`quota_bytes: null` = unbegrenzt).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Usage {
    pub used_bytes: u64,
    pub quota_bytes: Option<u64>,
}

impl Usage {
    /// Noch freier Platz; `None` = unbegrenzt.
    pub fn remaining(&self) -> Option<u64> {
        self.quota_bytes.map(|q| q.saturating_sub(self.used_bytes))
    }
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

pub async fn user_used(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<u64> {
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(size_bytes), 0)::BIGINT FROM attachments WHERE uploader_id = $1",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(to_u64(used))
}

pub async fn community_used(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<u64> {
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(size_bytes), 0)::BIGINT FROM attachments WHERE community_id = $1",
    )
    .bind(community_id)
    .fetch_one(db)
    .await?;
    Ok(to_u64(used))
}

pub async fn community_quota(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Option<u64>> {
    let mb: Option<i32> = sqlx::query_scalar("SELECT upload_quota_mb FROM communities WHERE id = $1")
        .bind(community_id)
        .fetch_optional(db)
        .await?
        .flatten();
    Ok(mb.map(|mb| u64::try_from(mb).unwrap_or(0) * MB))
}

/// Nutzer-Kontingent laut Instanz-Einstellungen (Laufzeit-Wert vor `LYKOS_USER_QUOTA_MB`).
pub async fn user_quota(state: &AppState) -> ApiResult<Option<u64>> {
    Ok(instance_settings::load(&state.db, &state.config).await?.user_quota_bytes)
}

pub async fn user_usage(state: &AppState, user_id: Uuid) -> ApiResult<Usage> {
    Ok(Usage { used_bytes: user_used(&state.db, user_id).await?, quota_bytes: user_quota(state).await? })
}

pub async fn community_usage(state: &AppState, community_id: Uuid) -> ApiResult<Usage> {
    Ok(Usage {
        used_bytes: community_used(&state.db, community_id).await?,
        quota_bytes: community_quota(&state.db, community_id).await?,
    })
}

/// Restplatz für einen neuen Upload (Minimum aus Nutzer- und Community-Kontingent); `None` = unbegrenzt.
/// Anhänge in Direktnachrichten (ohne Community) zählen nur zum Nutzer-Kontingent.
pub async fn remaining(
    state: &AppState,
    user_id: Uuid,
    community_id: Option<Uuid>,
) -> ApiResult<Option<u64>> {
    let user = user_usage(state, user_id).await?.remaining();
    let community = match community_id {
        Some(id) => community_usage(state, id).await?.remaining(),
        None => None,
    };
    Ok(match (user, community) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
}

/// Trägt einen gespeicherten Anhang ein, sofern er noch ins Kontingent passt
/// (sonst `quota_exceeded`; die Datei räumt der Aufrufer weg).
pub async fn insert_within_quota(state: &AppState, new: NewAttachment<'_>) -> ApiResult<Attachment> {
    let size = to_u64(new.size_bytes);
    let user_quota = user_quota(state).await?;
    let mut tx = state.db.begin().await?;
    // Feste Reihenfolge (erst Nutzer, dann Community) – so können sich zwei Uploads nicht gegenseitig blockieren.
    lock(&mut tx, "user", new.uploader_id).await?;
    if let Some(community_id) = new.community_id {
        lock(&mut tx, "community", community_id).await?;
    }

    if let Some(quota) = user_quota
        && user_used(&mut *tx, new.uploader_id).await?.saturating_add(size) > quota
    {
        return Err(exceeded());
    }
    if let Some(community_id) = new.community_id
        && let Some(quota) = community_quota(&mut *tx, community_id).await?
        && community_used(&mut *tx, community_id).await?.saturating_add(size) > quota
    {
        return Err(exceeded());
    }
    let attachment = attachments::insert(&mut *tx, new).await?;
    tx.commit().await?;
    Ok(attachment)
}

/// Transaktions-Sperre (Advisory-Lock) für ein Kontingent; endet mit der Transaktion.
async fn lock(conn: &mut PgConnection, kind: &str, id: Uuid) -> ApiResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("lykos-quota:{kind}:{id}"))
        .execute(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Usage;

    #[test]
    fn remaining_space() {
        assert_eq!(Usage { used_bytes: 10, quota_bytes: None }.remaining(), None);
        assert_eq!(Usage { used_bytes: 10, quota_bytes: Some(15) }.remaining(), Some(5));
        assert_eq!(Usage { used_bytes: 20, quota_bytes: Some(15) }.remaining(), Some(0));
    }
}
