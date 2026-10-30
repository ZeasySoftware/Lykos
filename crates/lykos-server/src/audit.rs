//! Audit-Log für administrative Aktionen innerhalb einer Community.

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::ApiResult, models::UserPublic};

pub async fn record(
    db: impl PgExecutor<'_>,
    community_id: Uuid,
    actor_id: Uuid,
    action: &str,
    target_id: Option<Uuid>,
    data: Value,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO audit_log (id, community_id, actor_id, action, target_id, data)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(community_id)
    .bind(actor_id)
    .bind(action)
    .bind(target_id)
    .bind(if data.is_null() { json!({}) } else { data })
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub actor: Option<UserPublic>,
    pub action: String,
    pub target_id: Option<Uuid>,
    pub data: Value,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    actor_id: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    action: String,
    target_id: Option<Uuid>,
    data: Value,
    created_at: OffsetDateTime,
}

pub async fn page(
    db: impl PgExecutor<'_>,
    community_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> ApiResult<Vec<AuditEntry>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.id, a.actor_id, u.username, u.display_name, u.avatar_id, a.action, a.target_id, a.data, a.created_at
         FROM audit_log a LEFT JOIN users u ON u.id = a.actor_id
         WHERE a.community_id = $1 AND ($2::uuid IS NULL OR a.id < $2)
         ORDER BY a.id DESC LIMIT $3",
    )
    .bind(community_id)
    .bind(before)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(Row::into_entry).collect())
}

impl Row {
    fn into_entry(self) -> AuditEntry {
        AuditEntry {
            id: self.id,
            actor: match (self.actor_id, self.username) {
                (Some(id), Some(username)) => {
                    Some(UserPublic::new(id, username, self.display_name, self.avatar_id))
                }
                _ => None,
            },
            action: self.action,
            target_id: self.target_id,
            data: self.data,
            created_at: self.created_at,
        }
    }
}

// --- Instanzweites Audit-Log (Paket H) ---------------------------------------
//
// Eigene Tabelle `instance_audit_log`: Admin-Aktionen sollen das Löschen der betroffenen Community
// oder des Nutzers überleben. `data` enthält daher die nötigen Namen (z. B. `username`) als Klartext.

/// Admin-Aktion auf Instanzebene protokollieren.
pub async fn record_instance(
    db: impl PgExecutor<'_>,
    actor_id: Uuid,
    action: &str,
    target_id: Option<Uuid>,
    data: Value,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO instance_audit_log (id, actor_id, action, target_id, data) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(actor_id)
    .bind(action)
    .bind(target_id)
    .bind(if data.is_null() { json!({}) } else { data })
    .execute(db)
    .await?;
    Ok(())
}

/// Seite des instanzweiten Audit-Logs (neueste zuerst, vor `before`).
pub async fn instance_page(
    db: impl PgExecutor<'_>,
    before: Option<Uuid>,
    limit: i64,
) -> ApiResult<Vec<AuditEntry>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.id, a.actor_id, u.username, u.display_name, u.avatar_id, a.action, a.target_id, a.data, a.created_at
         FROM instance_audit_log a LEFT JOIN users u ON u.id = a.actor_id
         WHERE ($1::uuid IS NULL OR a.id < $1)
         ORDER BY a.id DESC LIMIT $2",
    )
    .bind(before)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Row::into_entry).collect())
}
