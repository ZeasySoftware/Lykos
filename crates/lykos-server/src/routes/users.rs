//! Profile anderer Nutzer und Profilbilder (Avatare).
//!
//! Avatare werden vom Client vor dem Upload quadratisch auf höchstens 256 px verkleinert und neu
//! kodiert (Canvas) – das entfernt nebenbei Metadaten wie EXIF/GPS. Der Server dekodiert keine Bilder
//! (keine Bildbibliothek als Angriffsfläche/Abhängigkeit), prüft aber Typ (Magic Bytes), Größe und Maße.

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use futures_util::stream;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    attachments,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::{self, Scope},
    models::{self, User, UserProfile},
    routes::{attachments::multipart_error, relationships},
    state::AppState,
};

/// Höchstgröße eines Avatar-Uploads.
pub const AVATAR_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Größte zulässige Kantenlänge (schützt Clients vor riesigen Bildern).
pub const AVATAR_MAX_SIDE: i32 = 4096;

/// Antwort von `GET /users/{id}`: Profil plus die Communities, die man mit dem Nutzer teilt.
#[derive(Serialize)]
pub struct ProfileResponse {
    #[serde(flatten)]
    profile: UserProfile,
    /// Gemeinsame Communities (in Beitrittsreihenfolge des Abfragenden); beim eigenen Profil alle eigenen.
    mutual_community_ids: Vec<Uuid>,
}

/// `GET /users/{id}` – Profil eines Nutzers, mit dem man eine Community teilt (oder das eigene).
pub async fn profile(
    State(state): State<AppState>,
    user: AuthUser,
    Path(user_id): Path<Uuid>,
) -> ApiResult<Json<ProfileResponse>> {
    if !models::can_see_user(&state.db, user.user_id, user_id).await? {
        return Err(ApiError::not_found());
    }
    let target = models::user_by_id(&state.db, user_id).await?.ok_or_else(ApiError::not_found)?;
    let mutual_community_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT a.community_id FROM members a JOIN members b ON b.community_id = a.community_id
         WHERE a.user_id = $1 AND b.user_id = $2 ORDER BY a.joined_at",
    )
    .bind(user.user_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(ProfileResponse { profile: target.into(), mutual_community_ids }))
}

/// `POST /auth/me/avatar` – multipart mit genau einem Feld `file` (PNG, JPEG, WebP oder GIF).
pub async fn upload_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> ApiResult<Json<User>> {
    let mut field = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("file") => f,
        Ok(_) => return Err(ApiError::BadRequest("invalid_request")),
        Err(e) => return Err(multipart_error(e)),
    };
    let mut data = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        data.extend_from_slice(&chunk);
        if data.len() > AVATAR_MAX_BYTES {
            return Err(ApiError::PayloadTooLarge("file_too_large"));
        }
    }

    let content_type = attachments::detect_content_type(&data);
    let valid = attachments::image_dimensions(content_type, &data)
        .is_some_and(|(w, h)| (1..=AVATAR_MAX_SIDE).contains(&w) && (1..=AVATAR_MAX_SIDE).contains(&h));
    if !valid {
        return Err(ApiError::BadRequest("invalid_image"));
    }

    let avatar_id = Uuid::now_v7();
    let key = avatar_id.to_string();
    let body = Bytes::from(data);
    state
        .storage
        .put(&key, Box::pin(stream::once(async move { Ok(body) })))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Avatar speichern")))?;

    // Alte ID im selben Statement auslesen (der Self-Join sieht den Stand vor dem Update).
    let swapped: Result<Option<Option<Uuid>>, sqlx::Error> = sqlx::query_scalar(
        "UPDATE users u SET avatar_id = $2, avatar_content_type = $3
         FROM users old WHERE u.id = $1 AND old.id = u.id
         RETURNING old.avatar_id",
    )
    .bind(user.user_id)
    .bind(avatar_id)
    .bind(content_type)
    .fetch_optional(&state.db)
    .await;
    let old = match swapped {
        Ok(Some(old)) => old,
        Ok(None) => {
            attachments::delete_files(state.storage.as_ref(), &[avatar_id]).await;
            return Err(ApiError::Unauthorized("unauthorized"));
        }
        Err(e) => {
            attachments::delete_files(state.storage.as_ref(), &[avatar_id]).await;
            return Err(e.into());
        }
    };
    if let Some(old) = old {
        attachments::delete_files(state.storage.as_ref(), &[old]).await;
    }
    Ok(Json(announce_user_update(&state, user.user_id).await?))
}

/// `DELETE /auth/me/avatar` – Profilbild entfernen.
pub async fn delete_avatar(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<User>> {
    let old: Option<Option<Uuid>> = sqlx::query_scalar(
        "UPDATE users u SET avatar_id = NULL, avatar_content_type = NULL
         FROM users old WHERE u.id = $1 AND old.id = u.id
         RETURNING old.avatar_id",
    )
    .bind(user.user_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(Some(old)) = old {
        attachments::delete_files(state.storage.as_ref(), &[old]).await;
    }
    Ok(Json(announce_user_update(&state, user.user_id).await?))
}

/// `GET /users/{id}/avatar/{avatar_id}` – liefert das Bild. Nur mit Sichtbarkeit (siehe `can_see_user`)
/// und nur für die aktuelle Avatar-ID; alles andere ist `404`.
pub async fn avatar(
    State(state): State<AppState>,
    user: AuthUser,
    Path((user_id, avatar_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    if !models::can_see_user(&state.db, user.user_id, user_id).await? {
        return Err(ApiError::not_found());
    }
    let content_type: Option<Option<String>> =
        sqlx::query_scalar("SELECT avatar_content_type FROM users WHERE id = $1 AND avatar_id = $2")
            .bind(user_id)
            .bind(avatar_id)
            .fetch_optional(&state.db)
            .await?;
    let content_type = content_type
        .flatten()
        .filter(|t| attachments::INLINE_IMAGE_TYPES.contains(&t.as_str()))
        .ok_or_else(ApiError::not_found)?;

    let stream = state
        .storage
        .get(&avatar_id.to_string(), None)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Avatar lesen")))?
        .ok_or_else(ApiError::not_found)?;

    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // Neue Bilder bekommen eine neue ID → die URL ändert sich, Caches dürfen ewig halten.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    *res.status_mut() = StatusCode::OK;
    Ok(res)
}

/// Profiländerung verteilen: `USER_UPDATE` (Profil) an alle Communities und DM-Kanäle des Nutzers, an
/// Freunde und Angefragte (`relationships::profile_watchers`) und an seine eigenen Verbindungen,
/// zusätzlich `MEMBER_UPDATE` je Community (ältere Clients aktualisieren darüber Namen).
/// `USER_UPDATE` ist ein einziges Event an alle diese Scopes zusammen – jede Verbindung bekommt es
/// genau einmal, auch wenn sie mehrere Communities mit dem Nutzer teilt.
pub async fn announce_user_update(state: &AppState, user_id: Uuid) -> ApiResult<User> {
    let me = models::user_by_id(&state.db, user_id).await?.ok_or(ApiError::Unauthorized("unauthorized"))?;
    let profile = UserProfile::from(me.clone());

    let memberships = models::memberships_of(&state.db, user_id).await?;
    let mut scopes = Vec::with_capacity(memberships.len() + 1);
    for (community_id, member) in memberships {
        state.events.publish(
            events::community(community_id),
            "MEMBER_UPDATE",
            json!({ "community_id": community_id, "member": member }),
        );
        scopes.push(events::community(community_id));
    }
    let dms: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM dm_recipients WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;
    scopes.extend(dms.into_iter().map(Scope::Dm));
    // Freunde und Angefragte; wer den Nutzer blockiert oder ihn selbst angefragt hat, sieht sein Profil
    // nicht (`models::can_see_user`) und bekommt auch keine Änderungen.
    scopes.extend(relationships::profile_watchers(&state.db, user_id).await?.into_iter().map(Scope::User));
    scopes.push(Scope::User(user_id));
    state.events.publish_any(scopes, "USER_UPDATE", &profile);
    Ok(me)
}
