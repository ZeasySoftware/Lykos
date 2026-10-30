//! Instanz-Einstellungen, die Admins zur Laufzeit ändern können (Tabelle `instance_settings`).
//!
//! Die Umgebung (`Config`) liefert die Standardwerte; ein Eintrag in der Datenbank überschreibt sie.
//! Gelesen wird bei Bedarf direkt aus Postgres (eine kleine Abfrage) – so sehen mehrere Server-Instanzen
//! Änderungen sofort, ohne Cache-Invalidierung.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use sqlx::PgExecutor;

use crate::{config::Config, error::ApiResult};

const MB: u64 = 1024 * 1024;

/// Schlüssel in `instance_settings`.
pub mod keys {
    pub const INSTANCE_NAME: &str = "instance_name";
    pub const REGISTRATION_MODE: &str = "registration_mode";
    /// Maximale Upload-Größe in MB (> 0).
    pub const MAX_UPLOAD_MB: &str = "max_upload_mb";
    /// Standard-Kontingent pro Nutzer in MB (`0` = unbegrenzt).
    pub const USER_QUOTA_MB: &str = "user_quota_mb";
    pub const ALLOW_COMMUNITY_CREATION: &str = "allow_community_creation";
    pub const ANNOUNCEMENT: &str = "announcement";

    pub const ALL: [&str; 6] = [
        INSTANCE_NAME,
        REGISTRATION_MODE,
        MAX_UPLOAD_MB,
        USER_QUOTA_MB,
        ALLOW_COMMUNITY_CREATION,
        ANNOUNCEMENT,
    ];
}

pub const REGISTRATION_MODES: [&str; 3] = ["open", "invite_only", "closed"];
/// Standard-Registrierungsmodus, falls nichts eingetragen ist.
pub const DEFAULT_REGISTRATION_MODE: &str = "invite_only";

/// Effektive Einstellungen (Datenbank vor Umgebung).
#[derive(Debug, Clone, Serialize)]
pub struct Effective {
    pub instance_name: String,
    pub registration_mode: String,
    pub max_upload_bytes: u64,
    /// `None` = unbegrenzt.
    pub user_quota_bytes: Option<u64>,
    pub allow_community_creation: bool,
    /// Ankündigung, die allen angezeigt wird; `None` = keine.
    pub announcement: Option<String>,
}

/// Alle gespeicherten Einträge (Schlüssel → JSON-Wert).
pub async fn stored(db: impl PgExecutor<'_>) -> ApiResult<HashMap<String, Value>> {
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT key, value FROM instance_settings").fetch_all(db).await?;
    Ok(rows.into_iter().collect())
}

/// Effektive Einstellungen aus gespeicherten Werten und Umgebung.
pub fn resolve(config: &Config, stored: &HashMap<String, Value>) -> Effective {
    let mb = |key: &str| stored.get(key).and_then(Value::as_u64);
    Effective {
        instance_name: stored
            .get(keys::INSTANCE_NAME)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| config.instance_name.clone()),
        registration_mode: stored
            .get(keys::REGISTRATION_MODE)
            .and_then(Value::as_str)
            .filter(|m| REGISTRATION_MODES.contains(m))
            .unwrap_or(DEFAULT_REGISTRATION_MODE)
            .to_owned(),
        max_upload_bytes: mb(keys::MAX_UPLOAD_MB)
            .filter(|mb| *mb > 0)
            .map_or(config.max_upload_bytes, |mb| mb.saturating_mul(MB)),
        user_quota_bytes: match mb(keys::USER_QUOTA_MB) {
            Some(0) => None,
            Some(mb) => Some(mb.saturating_mul(MB)),
            None => config.user_quota_bytes,
        },
        allow_community_creation: stored
            .get(keys::ALLOW_COMMUNITY_CREATION)
            .and_then(Value::as_bool)
            .unwrap_or(true),
        announcement: stored
            .get(keys::ANNOUNCEMENT)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
    }
}

/// Effektive Einstellungen laden.
pub async fn load(db: impl PgExecutor<'_>, config: &Config) -> ApiResult<Effective> {
    Ok(resolve(config, &stored(db).await?))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn database_overrides_environment() {
        let config = Config::for_tests();
        let defaults = resolve(&config, &HashMap::new());
        assert_eq!(defaults.instance_name, "Test");
        assert_eq!(defaults.registration_mode, "invite_only");
        assert_eq!(defaults.max_upload_bytes, config.max_upload_bytes);
        assert_eq!(defaults.user_quota_bytes, config.user_quota_bytes);
        assert!(defaults.allow_community_creation);
        assert_eq!(defaults.announcement, None);

        let stored: HashMap<String, Value> = [
            (keys::INSTANCE_NAME, json!("Rudel")),
            (keys::REGISTRATION_MODE, json!("open")),
            (keys::MAX_UPLOAD_MB, json!(5)),
            (keys::USER_QUOTA_MB, json!(0)),
            (keys::ALLOW_COMMUNITY_CREATION, json!(false)),
            (keys::ANNOUNCEMENT, json!("  Wartung heute Abend  ")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let eff = resolve(&config, &stored);
        assert_eq!(eff.instance_name, "Rudel");
        assert_eq!(eff.registration_mode, "open");
        assert_eq!(eff.max_upload_bytes, 5 * MB);
        assert_eq!(eff.user_quota_bytes, None);
        assert!(!eff.allow_community_creation);
        assert_eq!(eff.announcement.as_deref(), Some("Wartung heute Abend"));
    }

    #[test]
    fn invalid_values_fall_back() {
        let config = Config::for_tests();
        let stored: HashMap<String, Value> = [
            (keys::REGISTRATION_MODE, json!("vielleicht")),
            (keys::MAX_UPLOAD_MB, json!(0)),
            (keys::ANNOUNCEMENT, json!("   ")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let eff = resolve(&config, &stored);
        assert_eq!(eff.registration_mode, "invite_only");
        assert_eq!(eff.max_upload_bytes, config.max_upload_bytes);
        assert_eq!(eff.announcement, None);
    }
}
