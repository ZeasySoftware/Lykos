//! Lykos-Server: HTTP-API, WebSocket-Gateway und Auslieferung des Web-Clients.

pub mod accounts;
pub mod admin;
pub mod attachments;
pub mod audit;
pub mod auth;
pub mod automod;
pub mod config;
pub mod desktop;
pub mod discord_template;
pub mod embeds;
pub mod emojis;
pub mod error;
pub mod events;
pub mod feedback;
pub mod gateway;
pub mod instance_settings;
pub mod livekit;
pub mod mail;
pub mod mentions;
pub mod mfa;
pub mod models;
pub mod oidc;
pub mod permissions;
pub mod presence;
pub mod quota;
pub mod rate_limit;
pub mod retention;
pub mod routes;
pub mod s3_storage;
pub mod security_headers;
pub mod state;
pub mod storage;
pub mod voice;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use sqlx::PgPool;
use tower_http::{
    compression::{CompressionLayer, Predicate, predicate::DefaultPredicate},
    cors::{AllowOrigin, Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::{config::Config, events::EventBus, state::AppState};

/// Migrationen, eingebettet in die Binary.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Anwendungszustand inkl. Speicher-Backend laut Konfiguration.
pub fn try_state(db: PgPool, config: Config) -> Result<AppState> {
    install_crypto_provider();
    admin::mark_started();
    let storage = storage::from_config(&config)?;
    let mailer = mail::from_config(&config.mail)?;
    Ok(AppState {
        db,
        config: Arc::new(config),
        events: EventBus::default(),
        rate_limit: rate_limit::RateLimiter::default(),
        storage,
        presence: Default::default(),
        desktop: Default::default(),
        mailer,
        oidc: Default::default(),
        automod: Default::default(),
    })
}

/// Wie `try_state`; panikt bei ungültiger Speicher-Konfiguration (für Tests).
pub fn state(db: PgPool, config: Config) -> AppState {
    try_state(db, config).expect("Speicher-Backend")
}

/// rustls-Kryptoprovider für ausgehende HTTPS-Aufrufe (Release-API, LiveKit, S3; reqwest ohne eigenen
/// Provider, `ring` steckt über sqlx ohnehin im Build). Einzige Stelle dafür; mehrfacher Aufruf ist harmlos.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Komplette Anwendung: API unter `/api`, optional der Web-Client als Fallback.
pub fn app(state: AppState) -> Result<Router> {
    let mut app = Router::new().nest("/api", routes::api());

    // Web-Client ausliefern; unbekannte Pfade fallen auf index.html zurück (SPA-Routing).
    if let Some(dir) = &state.config.web_dir {
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
        tracing::info!(dir = %dir.display(), "Web-Client wird ausgeliefert");
    }

    // CSP, Framing, Referrer, nosniff, ggf. HSTS – für Web-Client und API (siehe `security_headers`).
    let headers = security_headers::SecurityHeaders::from_config(&state.config)?;
    let app = app.layer(axum::middleware::map_response(move |res: axum::response::Response| {
        let headers = headers.clone();
        async move { headers.apply(res) }
    }));

    let cors = cors(&state.config.cors_origins)?;
    Ok(app
        .layer(cors)
        .layer(middleware::from_fn(static_cache_control))
        .layer(compression())
        // Wie `TraceLayer::new_for_http()`, aber ohne Query und mit geschwärztem Webhook-Token im Pfad.
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request| {
            tracing::debug_span!(
                "request",
                method = %req.method(),
                path = %redact_path(req.uri().path()),
                version = ?req.version(),
            )
        }))
        .with_state(state))
}

/// Pfad fürs Log: Das Token in `/api/webhooks/{id}/{token}…` wird durch `***` ersetzt (Tokens landen nie
/// im Log). Verwaltungs-Pfade (`…/avatar`, `…/token`) bleiben unverändert.
pub fn redact_path(path: &str) -> std::borrow::Cow<'_, str> {
    let Some(rest) = path.strip_prefix("/api/webhooks/") else {
        return path.into();
    };
    let mut parts = rest.splitn(3, '/');
    let (Some(id), Some(token)) = (parts.next(), parts.next()) else {
        return path.into();
    };
    if matches!(token, "avatar" | "token" | "") {
        return path.into();
    }
    match parts.next() {
        Some(tail) => format!("/api/webhooks/{id}/***/{tail}").into(),
        None => format!("/api/webhooks/{id}/***").into(),
    }
}

/// gzip/brotli für API-Antworten und den Web-Client. Ausgenommen (neben Bildern, SSE und Winzlingen
/// laut `DefaultPredicate`): WebSocket-Upgrades, Teilantworten (Range) und Dateien aus dem Speicher
/// (`Content-Disposition`: Anhänge mit Range-Unterstützung, Avatare, Emojis, Export) – die sind
/// meist schon komprimiert, und Range-Anfragen müssen sich auf die unkomprimierten Bytes beziehen.
fn compression() -> CompressionLayer<impl Predicate> {
    let predicate = DefaultPredicate::new().and(|status: StatusCode, _, headers: &HeaderMap, _: &_| {
        status != StatusCode::SWITCHING_PROTOCOLS
            && status != StatusCode::PARTIAL_CONTENT
            && !headers.contains_key(header::CONTENT_RANGE)
            && !headers.contains_key(header::CONTENT_DISPOSITION)
    });
    CompressionLayer::new().gzip(true).br(true).compress_when(predicate)
}

/// Cache-Header für den Web-Client: Vite-Dateien unter `/assets/` tragen einen Inhalts-Hash im Namen
/// und dürfen ewig gecacht werden; alles andere (v. a. `index.html`, auch als SPA-Fallback) wird bei
/// jedem Laden neu geprüft. Die API setzt ihre Cache-Header selbst.
async fn static_cache_control(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return next.run(req).await;
    }
    let assets = path.starts_with("/assets/");
    let mut res = next.run(req).await;
    // Fehlt eine Datei unter `/assets/`, liefert der SPA-Fallback index.html – die darf nicht hängen bleiben.
    let html =
        res.headers().get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    let value = if assets && !html && (res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED)
    {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    res
}

fn cors(origins: &[String]) -> Result<CorsLayer> {
    let origins = origins
        .iter()
        .map(|o| HeaderValue::from_str(o).with_context(|| format!("ungültiger CORS-Origin: {o}")))
        .collect::<Result<Vec<_>>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]))
}

#[cfg(test)]
mod tests {
    use super::redact_path;

    #[test]
    fn webhook_tokens_are_redacted_in_logs() {
        assert_eq!(redact_path("/api/webhooks/abc/lykw_geheim"), "/api/webhooks/abc/***");
        assert_eq!(
            redact_path("/api/webhooks/abc/lykw_geheim/messages/m1"),
            "/api/webhooks/abc/***/messages/m1"
        );
        assert_eq!(redact_path("/api/webhooks/abc/avatar/x"), "/api/webhooks/abc/avatar/x");
        assert_eq!(redact_path("/api/webhooks/abc/token"), "/api/webhooks/abc/token");
        assert_eq!(redact_path("/api/webhooks/abc"), "/api/webhooks/abc");
        assert_eq!(redact_path("/api/messages/x"), "/api/messages/x");
    }
}
