//! Rechtstexte der Instanz (Impressum, Datenschutzerklärung) – öffentlich, ohne Anmeldung.

use axum::{Json, extract::State};
use serde::Serialize;

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Ein Rechtstext als Markdown; der Client rendert ihn ohne HTML (siehe `client/src/lib/releaseNotes.ts`).
#[derive(Serialize)]
pub struct LegalText {
    content: String,
}

fn text(content: &Option<String>) -> ApiResult<Json<LegalText>> {
    content.clone().map(|content| Json(LegalText { content })).ok_or_else(ApiError::not_found)
}

/// `GET /api/legal/imprint` – Impressum (`LYKOS_IMPRINT_FILE`), sonst 404.
pub async fn imprint(State(state): State<AppState>) -> ApiResult<Json<LegalText>> {
    text(&state.config.legal.imprint)
}

/// `GET /api/legal/privacy` – Datenschutzerklärung (`LYKOS_PRIVACY_FILE`), sonst 404.
pub async fn privacy(State(state): State<AppState>) -> ApiResult<Json<LegalText>> {
    text(&state.config.legal.privacy)
}
