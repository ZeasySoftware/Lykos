//! Instanz-Einstellungen zur Laufzeit (`instance_settings`); Umgebungsvariablen bleiben die Standardwerte.
//! Änderungen gehen als `INSTANCE_UPDATE` an alle Gateway-Verbindungen.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    admin::InstanceAdmin,
    audit, auth,
    error::{ApiError, ApiResult},
    events::Scope,
    instance_settings::{self, REGISTRATION_MODES, keys},
    models::double_option,
    routes::instance,
    state::AppState,
};

const MB: u64 = 1024 * 1024;
/// Obergrenze Upload-Größe (100 GB) – schützt nur vor Tippfehlern.
const MAX_UPLOAD_MB_LIMIT: u64 = 100 * 1024;
/// Obergrenze Nutzer-Kontingent (≈ 10 TB).
const USER_QUOTA_MB_LIMIT: u64 = 10_000_000;
const NAME_MAX_CHARS: usize = 64;
const ANNOUNCEMENT_MAX_CHARS: usize = 500;

#[derive(Serialize)]
pub struct Defaults {
    instance_name: String,
    max_upload_mb: u64,
    /// `null` = unbegrenzt.
    user_quota_mb: Option<u64>,
}

#[derive(Serialize)]
pub struct SettingsView {
    instance_name: String,
    registration_mode: String,
    max_upload_mb: u64,
    /// `null` = unbegrenzt.
    user_quota_mb: Option<u64>,
    allow_community_creation: bool,
    announcement: Option<String>,
    /// Werte aus der Umgebung, die bei „Zurücksetzen“ gelten.
    defaults: Defaults,
    /// Schlüssel, die in der Datenbank überschrieben sind.
    overridden: Vec<String>,
}

async fn view(state: &AppState) -> ApiResult<SettingsView> {
    let stored = instance_settings::stored(&state.db).await?;
    let eff = instance_settings::resolve(&state.config, &stored);
    let mut overridden: Vec<String> =
        keys::ALL.iter().filter(|k| stored.contains_key(**k)).map(|k| (*k).to_owned()).collect();
    overridden.sort();
    Ok(SettingsView {
        instance_name: eff.instance_name,
        registration_mode: eff.registration_mode,
        max_upload_mb: eff.max_upload_bytes / MB,
        user_quota_mb: eff.user_quota_bytes.map(|b| b / MB),
        allow_community_creation: eff.allow_community_creation,
        announcement: eff.announcement,
        defaults: Defaults {
            instance_name: state.config.instance_name.clone(),
            max_upload_mb: state.config.max_upload_bytes / MB,
            user_quota_mb: state.config.user_quota_bytes.map(|b| b / MB),
        },
        overridden,
    })
}

/// `GET /admin/settings`
pub async fn get(State(state): State<AppState>, _admin: InstanceAdmin) -> ApiResult<Json<SettingsView>> {
    Ok(Json(view(&state).await?))
}

/// Felder fehlen = unverändert; `null` = auf den Standard (Umgebung) zurücksetzen bzw. entfernen.
#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(default, deserialize_with = "double_option")]
    instance_name: Option<Option<String>>,
    registration_mode: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    max_upload_mb: Option<Option<u64>>,
    /// `0` = unbegrenzt.
    #[serde(default, deserialize_with = "double_option")]
    user_quota_mb: Option<Option<u64>>,
    #[serde(default, deserialize_with = "double_option")]
    allow_community_creation: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option")]
    announcement: Option<Option<String>>,
}

/// Geprüfte Änderungen: Schlüssel → neuer Wert (`None` = Eintrag löschen).
fn validate(body: UpdateBody) -> ApiResult<Vec<(&'static str, Option<Value>)>> {
    let mut changes = Vec::new();
    if let Some(name) = body.instance_name {
        let name = match name.map(|n| n.trim().to_owned()) {
            Some(n) if n.is_empty() || n.chars().count() > NAME_MAX_CHARS => {
                return Err(ApiError::BadRequest("invalid_name"));
            }
            other => other,
        };
        changes.push((keys::INSTANCE_NAME, name.map(Value::from)));
    }
    if let Some(mode) = body.registration_mode {
        if !REGISTRATION_MODES.contains(&mode.as_str()) {
            return Err(ApiError::BadRequest("invalid_registration_mode"));
        }
        changes.push((keys::REGISTRATION_MODE, Some(Value::from(mode))));
    }
    if let Some(mb) = body.max_upload_mb {
        if mb.is_some_and(|mb| !(1..=MAX_UPLOAD_MB_LIMIT).contains(&mb)) {
            return Err(ApiError::BadRequest("invalid_request"));
        }
        changes.push((keys::MAX_UPLOAD_MB, mb.map(Value::from)));
    }
    if let Some(mb) = body.user_quota_mb {
        if mb.is_some_and(|mb| mb > USER_QUOTA_MB_LIMIT) {
            return Err(ApiError::BadRequest("invalid_request"));
        }
        changes.push((keys::USER_QUOTA_MB, mb.map(Value::from)));
    }
    if let Some(allow) = body.allow_community_creation {
        changes.push((keys::ALLOW_COMMUNITY_CREATION, allow.map(Value::from)));
    }
    if let Some(text) = body.announcement {
        let text = auth::normalize_text(text, ANNOUNCEMENT_MAX_CHARS, "invalid_announcement")?;
        changes.push((keys::ANNOUNCEMENT, text.map(Value::from)));
    }
    Ok(changes)
}

/// `PATCH /admin/settings`
pub async fn update(
    State(state): State<AppState>,
    admin: InstanceAdmin,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<SettingsView>> {
    let changes = validate(body)?;
    if changes.is_empty() {
        return Ok(Json(view(&state).await?));
    }

    let mut tx = state.db.begin().await?;
    let mut logged = Map::new();
    for (key, value) in &changes {
        match value {
            Some(value) => {
                sqlx::query(
                    "INSERT INTO instance_settings (key, value) VALUES ($1, $2)
                     ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
                )
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM instance_settings WHERE key = $1")
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        logged.insert((*key).to_owned(), value.clone().unwrap_or(Value::Null));
    }
    audit::record_instance(&mut *tx, admin.user_id(), "settings.update", None, Value::Object(logged)).await?;
    tx.commit().await?;

    state.events.publish(Scope::Instance, "INSTANCE_UPDATE", json!(instance::info(&state).await?));
    Ok(Json(view(&state).await?))
}
