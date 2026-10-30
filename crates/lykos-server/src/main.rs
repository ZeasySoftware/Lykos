use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use lykos_server::{
    MIGRATOR, attachments,
    config::{Config, StorageBackend},
    rate_limit, retention, voice,
};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "lykos_server=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!(bind = %config.bind, public_url = %config.public_url, "Lykos startet");
    if config.trust_proxy && config.trusted_proxies.is_empty() {
        tracing::warn!(
            "LYKOS_TRUST_PROXY=true ohne LYKOS_TRUSTED_PROXIES: X-Forwarded-For gilt von jeder Verbindung – wer den \
             Port direkt erreicht, kann die IP-Limits umgehen. Adresse des Reverse Proxys in LYKOS_TRUSTED_PROXIES eintragen."
        );
    }

    let db = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("Verbindung zur Datenbank fehlgeschlagen")?;

    MIGRATOR.run(&db).await.context("Migrationen fehlgeschlagen")?;
    retention::spawn(db.clone());

    match &config.storage {
        StorageBackend::Local => {
            tokio::fs::create_dir_all(&config.storage_dir).await.with_context(|| {
                format!("Speicherverzeichnis {} nicht anlegbar", config.storage_dir.display())
            })?;
            tracing::info!(dir = %config.storage_dir.display(), "Dateispeicher: lokal");
        }
        StorageBackend::S3(s3) => {
            tracing::info!(endpoint = ?s3.endpoint, bucket = %s3.bucket, prefix = ?s3.prefix, "Dateispeicher: S3");
        }
    }

    let bind = config.bind;
    let state = lykos_server::try_state(db, config)?;
    rate_limit::spawn_cleanup(state.rate_limit.clone());
    attachments::spawn_cleanup(state.db.clone(), state.storage.clone());
    voice::spawn_startup_sync(state.clone());
    let app = lykos_server::app(state)?;

    let listener = tokio::net::TcpListener::bind(bind).await?;
    // ConnectInfo liefert die Socket-Adresse für das Rate-Limiting (ohne Proxy).
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Lykos beendet");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("Ctrl+C-Handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM-Handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
