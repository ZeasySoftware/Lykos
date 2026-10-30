use axum::{Json, extract::State};
use serde::Serialize;

use crate::{error::ApiResult, instance_settings, state::AppState};

/// Öffentliche Eckdaten der Instanz, die der Client vor dem Login braucht.
/// Werte aus `instance_settings` haben Vorrang vor der Umgebung (siehe `instance_settings.rs`).
#[derive(Serialize)]
pub struct InstanceInfo {
    name: String,
    public_url: String,
    registration_mode: String,
    /// Noch kein Nutzer registriert – der erste wird Instanz-Admin.
    bootstrap: bool,
    /// Maximale Größe eines Datei-Uploads in Bytes.
    max_upload_bytes: u64,
    /// Voice/Screenshare verfügbar (LiveKit konfiguriert).
    voice: bool,
    /// Dürfen alle Nutzer Communities anlegen (sonst nur Instanz-Admins)?
    allow_community_creation: bool,
    /// Ankündigung der Admins, die allen angezeigt wird; `null` = keine.
    announcement: Option<String>,
    /// Bot-Konten erlaubt (Einstellungs-Tab „Entwickler“, Bots zu Communities hinzufügen).
    allow_bots: bool,
    /// Mailversand eingerichtet: E-Mail-Adresse, „Passwort vergessen“ und 2FA per E-Mail verfügbar.
    email: bool,
    /// Konfigurierte Anmeldeanbieter („Mit … anmelden“).
    oidc_providers: Vec<OidcProviderInfo>,
    /// Quellcode der laufenden Fassung (AGPL-3.0, `LYKOS_SOURCE_URL`).
    source_url: String,
    /// Welche Rechtstexte hinterlegt sind (`GET /api/legal/imprint` bzw. `/privacy`).
    legal: LegalInfo,
}

#[derive(Serialize)]
pub struct LegalInfo {
    imprint: bool,
    privacy: bool,
}

#[derive(Serialize)]
pub struct OidcProviderInfo {
    id: String,
    label: String,
}

/// Aktuelle Instanzdaten (auch Inhalt des Gateway-Events `INSTANCE_UPDATE`).
pub async fn info(state: &AppState) -> ApiResult<InstanceInfo> {
    let bootstrap: bool =
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM users)").fetch_one(&state.db).await?;
    let settings = instance_settings::load(&state.db, &state.config).await?;

    Ok(InstanceInfo {
        name: settings.instance_name,
        public_url: state.config.public_url.clone(),
        registration_mode: settings.registration_mode,
        bootstrap,
        max_upload_bytes: settings.max_upload_bytes,
        voice: state.config.livekit.is_some(),
        allow_community_creation: settings.allow_community_creation,
        announcement: settings.announcement,
        allow_bots: settings.allow_bots,
        email: state.mailer.enabled(),
        oidc_providers: state
            .config
            .oidc_providers
            .iter()
            .map(|p| OidcProviderInfo { id: p.id.clone(), label: p.label.clone() })
            .collect(),
        source_url: state.config.source_url.clone(),
        legal: LegalInfo {
            imprint: state.config.legal.imprint.is_some(),
            privacy: state.config.legal.privacy.is_some(),
        },
    })
}

pub async fn instance(State(state): State<AppState>) -> ApiResult<Json<InstanceInfo>> {
    Ok(Json(info(&state).await?))
}
