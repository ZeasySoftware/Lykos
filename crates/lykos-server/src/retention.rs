//! Hintergrund-Job: löscht Nachrichten nach Ablauf der Aufbewahrungsfrist ihrer Community
//! und räumt abgelaufene Sessions auf. Mehrfaches Laufen (mehrere Instanzen) ist unschädlich.

use std::time::Duration;

use sqlx::PgPool;

const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Nachrichten pro Lösch-Transaktion.
const MESSAGE_BATCH: i64 = 5_000;

pub fn spawn(db: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(INTERVAL);
        loop {
            tick.tick().await;
            if let Err(e) = run_once(&db).await {
                tracing::warn!(error = %e, "Aufbewahrungs-Job fehlgeschlagen");
            }
        }
    });
}

pub async fn run_once(db: &PgPool) -> sqlx::Result<()> {
    let messages = delete_expired_messages(db, MESSAGE_BATCH).await?;

    let sessions =
        sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(db).await?.rows_affected();

    // Abgelaufene Einmal-Tokens und -Codes (Paket G); Wiederherstellungscodes bleiben bis zum Neuerzeugen.
    for query in [
        "DELETE FROM email_verifications WHERE expires_at < now()",
        "DELETE FROM password_reset_tokens WHERE expires_at < now()",
        "DELETE FROM mfa_email_codes WHERE expires_at < now()",
        "DELETE FROM mfa_challenges WHERE expires_at < now()",
        "DELETE FROM oidc_flows WHERE expires_at < now()",
        "DELETE FROM oidc_signups WHERE expires_at < now()",
    ] {
        sqlx::query(query).execute(db).await?;
    }

    if messages > 0 || sessions > 0 {
        tracing::info!(messages, sessions, "Aufbewahrung: abgelaufene Daten gelöscht");
    }
    Ok(())
}

/// Abgelaufene Nachrichten in Portionen von `batch` löschen. Jede Portion ist eine eigene kurze
/// Transaktion – so sperrt das erste Aufräumen nach dem Setzen einer Frist (evtl. Hunderttausende
/// Nachrichten samt Kaskaden auf Reaktionen/Erwähnungen) nicht minutenlang. Liefert die Gesamtzahl.
pub async fn delete_expired_messages(db: &PgPool, batch: i64) -> sqlx::Result<u64> {
    let mut total = 0;
    loop {
        let deleted = sqlx::query(
            "WITH doomed AS (
                 SELECT m.id FROM messages m
                 JOIN channels c ON c.id = m.channel_id
                 JOIN communities co ON co.id = c.community_id
                 WHERE co.message_retention_days IS NOT NULL
                   AND m.created_at < now() - make_interval(days => co.message_retention_days)
                 LIMIT $1
             )
             DELETE FROM messages WHERE id IN (SELECT id FROM doomed)",
        )
        .bind(batch)
        .execute(db)
        .await?
        .rows_affected();
        total += deleted;
        if deleted < batch as u64 {
            return Ok(total);
        }
    }
}
