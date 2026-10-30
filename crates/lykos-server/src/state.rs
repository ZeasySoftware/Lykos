use std::sync::Arc;

use sqlx::PgPool;

use crate::{
    config::Config, desktop::DesktopReleases, events::EventBus, mail::Mailer, oidc::Oidc, presence::Presence,
    rate_limit::RateLimiter, storage::Storage,
};

/// Geteilter Zustand aller Handler. Bewusst ohne eigenen fachlichen In-Memory-Zustand,
/// damit mehrere Server-Instanzen hinter einem Load Balancer laufen können
/// (der Event-Bus wird dafür später über NATS verbunden).
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub events: EventBus,
    /// Ausnahme vom Grundsatz oben: Anmelde-/Registrierungszähler liegen vorerst im Speicher
    /// (eine Instanz im Pilot); bei mehreren Instanzen nach Redis/Postgres verlagern.
    pub rate_limit: RateLimiter,
    /// Speicher für Dateianhänge (lokal, später S3).
    pub storage: Arc<dyn Storage>,
    /// Flüchtige Präsenz/Tipp-Drosselung. Einzige Ausnahme von der Regel oben: lebt pro Prozess
    /// und muss für mehrere Instanzen in einen geteilten Speicher wandern (siehe `presence.rs`).
    pub presence: Presence,
    /// Cache für das neueste Desktop-Release (reiner Cache, darf pro Instanz abweichen).
    pub desktop: Arc<DesktopReleases>,
    /// Mailversand (deaktiviert, Log oder SMTP; in Tests ein sammelnder Mailer).
    pub mailer: Arc<dyn Mailer>,
    /// OIDC-Client mit Cache für Discovery und Schlüssel der Anbieter.
    pub oidc: Arc<Oidc>,
}
