//! Öffentliche Endpunkte für die Desktop-App: Download-Seite und Tauri-Update-Manifest.

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{
    desktop::Unavailable,
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Clients und Proxys dürfen kurz cachen; der Server cacht selbst länger.
const CACHE_CONTROL: HeaderValue = HeaderValue::from_static("public, max-age=300");

/// Neuester Windows-Installer: `{ version, notes, published_at, release_url, file_name,
/// download_url, size, sha256 }`. 404 `no_release` ohne Konfiguration oder passendes Release.
pub async fn latest(State(state): State<AppState>) -> ApiResult<Response> {
    let url = state.config.desktop_releases_url.as_deref().ok_or(ApiError::NotFound("no_release"))?;
    let release = state
        .desktop
        .latest(url)
        .await
        .map_err(|Unavailable| ApiError::ServiceUnavailable("desktop_unavailable"))?
        .ok_or(ApiError::NotFound("no_release"))?;
    Ok(([(header::CACHE_CONTROL, CACHE_CONTROL)], Json(release.as_ref().clone())).into_response())
}

/// Statisches Tauri-Update-Manifest (`latest.json`). Ohne Release oder ohne Signatur `204`
/// (= „kein Update“ für den Tauri-Updater).
pub async fn update_manifest(State(state): State<AppState>) -> ApiResult<Response> {
    let Some(url) = state.config.desktop_releases_url.as_deref() else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let release = state
        .desktop
        .latest(url)
        .await
        .map_err(|Unavailable| ApiError::ServiceUnavailable("desktop_unavailable"))?;
    match release.and_then(|r| r.update_manifest()) {
        Some(manifest) => Ok(([(header::CACHE_CONTROL, CACHE_CONTROL)], Json(manifest)).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
