//! Paket J: Community- und Profil-Gestaltung – Community-Icon und -Banner, Profilbanner (Hochladen,
//! Entfernen, Abrufen), Farbprüfung und Beitritts-Systemnachrichten.
//!
//! Wie bei Avataren schneidet und skaliert der Client die Bilder vor dem Upload und kodiert sie neu
//! (Canvas → WebP/PNG, entfernt EXIF/GPS). Der Server dekodiert keine Bilder, prüft aber Typ
//! (Magic Bytes), Größe und Kantenlängen. Jeder Upload bekommt eine neue ID → die URL ist unveränderlich
//! und darf ewig gecacht werden; das alte Bild wird sofort gelöscht.
//!
//! Sichtbarkeit: Community-Bilder sehen Mitglieder (`/communities/{id}/icon|banner/{bild_id}`) und – für die
//! öffentliche Einladungsseite – alle mit einem gültigen Einladungscode (`/invites/{code}/icon|banner/{bild_id}`).
//! Profilbanner sehen dieselben Nutzer wie den Avatar (`models::can_see_user`).

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use futures_util::stream;
use serde_json::json;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events,
    models::{self, Community, User},
    permissions::{self, Permissions},
    routes::{attachments::multipart_error, messages, users},
    state::AppState,
};

/// Höchstlänge der Community-Beschreibung (Zeichen).
pub const DESCRIPTION_MAX_CHARS: usize = 300;
/// Höchstlänge der Pronomen (Zeichen).
pub const PRONOUNS_MAX_CHARS: usize = 40;

/// Community-Icon: der Client liefert 256 px (animierte GIFs bis 1 MB unverändert).
pub const ICON_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const ICON_MAX_SIDE: i32 = 1024;
/// Banner (Community 960×540, Profil 600×240 – großzügig begrenzt).
pub const BANNER_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const BANNER_MAX_SIDE: i32 = 2048;

/// Zusätzlicher Spielraum für Multipart-Rahmen im Body-Limit.
pub const MULTIPART_OVERHEAD: usize = 64 * 1024;

/// Unveränderliche URLs (neue ID je Upload) für angemeldete Nutzer.
const CACHE_PRIVATE: &str = "private, max-age=31536000, immutable";
/// Bilder der Einladungsseite: die URL bleibt gleich, die Einladung kann aber ablaufen.
const CACHE_INVITE: &str = "private, max-age=3600";

/// Farbe als 24-Bit-RGB (`0xRRGGBB`); `None` = Standard.
pub fn validate_color(color: Option<i32>) -> ApiResult<()> {
    match color {
        Some(c) if !(0..=0xFF_FFFF).contains(&c) => Err(ApiError::BadRequest("invalid_color")),
        _ => Ok(()),
    }
}

// --- Hochladen, Speichern, Ausliefern -----------------------------------------

struct ImageUpload {
    data: Vec<u8>,
    content_type: &'static str,
}

/// Liest genau ein Multipart-Feld `file` und prüft Typ (PNG/JPEG/WebP/GIF), Größe und Maße.
async fn read_image(mut multipart: Multipart, max_bytes: usize, max_side: i32) -> ApiResult<ImageUpload> {
    let mut field = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("file") => f,
        Ok(_) => return Err(ApiError::BadRequest("invalid_request")),
        Err(e) => return Err(multipart_error(e)),
    };
    let mut data = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        data.extend_from_slice(&chunk);
        if data.len() > max_bytes {
            return Err(ApiError::PayloadTooLarge("file_too_large"));
        }
    }
    let content_type = attachments::detect_content_type(&data);
    let valid = attachments::image_dimensions(content_type, &data)
        .is_some_and(|(w, h)| (1..=max_side).contains(&w) && (1..=max_side).contains(&h));
    if !valid {
        return Err(ApiError::BadRequest("invalid_image"));
    }
    Ok(ImageUpload { data, content_type })
}

/// Legt das Bild unter einer neuen ID im Speicher ab.
async fn store(state: &AppState, image: Vec<u8>) -> ApiResult<Uuid> {
    let id = Uuid::now_v7();
    let key = id.to_string();
    let body = Bytes::from(image);
    state
        .storage
        .put(&key, Box::pin(stream::once(async move { Ok(body) })))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Bild speichern")))?;
    Ok(id)
}

/// Liefert ein gespeichertes Bild aus (nur die vier Inline-Bildtypen).
async fn serve(
    state: &AppState,
    id: Uuid,
    content_type: Option<String>,
    cache_control: &'static str,
) -> ApiResult<Response> {
    let content_type = content_type
        .filter(|t| attachments::INLINE_IMAGE_TYPES.contains(&t.as_str()))
        .ok_or_else(ApiError::not_found)?;
    let stream = state
        .storage
        .get(&id.to_string(), None)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Bild lesen")))?
        .ok_or_else(ApiError::not_found)?;

    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
    *res.status_mut() = StatusCode::OK;
    Ok(res)
}

// --- Community-Icon und -Banner -------------------------------------------------

#[derive(Clone, Copy)]
enum CommunityImage {
    Icon,
    Banner,
}

impl CommunityImage {
    fn limits(self) -> (usize, i32) {
        match self {
            Self::Icon => (ICON_MAX_BYTES, ICON_MAX_SIDE),
            Self::Banner => (BANNER_MAX_BYTES, BANNER_MAX_SIDE),
        }
    }

    /// Setzt (`$2`, `$3`) bzw. entfernt (beide `NULL`) das Bild und liefert die bisherige ID.
    /// Der Self-Join sieht den Stand vor dem Update.
    fn swap_sql(self) -> &'static str {
        match self {
            Self::Icon => {
                "UPDATE communities c SET icon_id = $2, icon_content_type = $3
                 FROM communities old WHERE c.id = $1 AND old.id = c.id RETURNING old.icon_id"
            }
            Self::Banner => {
                "UPDATE communities c SET banner_id = $2, banner_content_type = $3
                 FROM communities old WHERE c.id = $1 AND old.id = c.id RETURNING old.banner_id"
            }
        }
    }

    fn content_type_sql(self) -> &'static str {
        match self {
            Self::Icon => "SELECT icon_content_type FROM communities WHERE id = $1 AND icon_id = $2",
            Self::Banner => "SELECT banner_content_type FROM communities WHERE id = $1 AND banner_id = $2",
        }
    }

    /// Bild einer Community über einen gültigen Einladungscode – Community-Einladung oder Instanz-Einladung
    /// mit automatischem Beitritt zu dieser Community (Paket H).
    fn invite_sql(self) -> &'static str {
        macro_rules! by_invite {
            ($content_type:literal, $id:literal) => {
                concat!(
                    "SELECT c.",
                    $content_type,
                    " FROM communities c WHERE c.",
                    $id,
                    " = $2 AND (
                       EXISTS (SELECT 1 FROM invites i WHERE i.community_id = c.id AND i.code = $1
                                 AND (i.expires_at IS NULL OR i.expires_at > now())
                                 AND (i.max_uses IS NULL OR i.uses < i.max_uses))
                    OR EXISTS (SELECT 1 FROM instance_invites i WHERE i.community_id = c.id AND i.code = $1
                                 AND i.revoked_at IS NULL
                                 AND (i.expires_at IS NULL OR i.expires_at > now())
                                 AND (i.max_uses IS NULL OR i.uses < i.max_uses)))"
                )
            };
        }
        match self {
            Self::Icon => by_invite!("icon_content_type", "icon_id"),
            Self::Banner => by_invite!("banner_content_type", "banner_id"),
        }
    }

    fn audit_action(self, set: bool) -> &'static str {
        match (self, set) {
            (Self::Icon, true) => "community.icon_set",
            (Self::Icon, false) => "community.icon_remove",
            (Self::Banner, true) => "community.banner_set",
            (Self::Banner, false) => "community.banner_remove",
        }
    }
}

/// Setzt (`Some`) oder entfernt (`None`) ein Community-Bild. Bei Fehlern bleibt kein neues Bild liegen.
async fn swap_community_image(
    state: &AppState,
    user: AuthUser,
    community_id: Uuid,
    kind: CommunityImage,
    image: Option<(Uuid, &'static str)>,
) -> ApiResult<Json<Community>> {
    let result = async {
        let mut tx = state.db.begin().await?;
        let old: Option<Option<Uuid>> = sqlx::query_scalar(kind.swap_sql())
            .bind(community_id)
            .bind(image.map(|i| i.0))
            .bind(image.map(|i| i.1))
            .fetch_optional(&mut *tx)
            .await?;
        let old = old.ok_or_else(ApiError::not_found)?;
        if image.is_some() || old.is_some() {
            audit::record(
                &mut *tx,
                community_id,
                user.user_id,
                kind.audit_action(image.is_some()),
                None,
                json!({}),
            )
            .await?;
        }
        tx.commit().await?;
        Ok::<_, ApiError>(old)
    }
    .await;
    let old = match result {
        Ok(old) => old,
        Err(e) => {
            if let Some((id, _)) = image {
                attachments::delete_files(state.storage.as_ref(), &[id]).await;
            }
            return Err(e);
        }
    };
    if let Some(old) = old {
        attachments::delete_files(state.storage.as_ref(), &[old]).await;
    }

    let community =
        models::community_by_id(&state.db, community_id).await?.ok_or_else(ApiError::not_found)?;
    state.events.publish(events::community(community_id), "COMMUNITY_UPDATE", &community);
    Ok(Json(community))
}

async fn upload_community_image(
    state: AppState,
    user: AuthUser,
    community_id: Uuid,
    kind: CommunityImage,
    multipart: Multipart,
) -> ApiResult<Json<Community>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;
    let (max_bytes, max_side) = kind.limits();
    let image = read_image(multipart, max_bytes, max_side).await?;
    let id = store(&state, image.data).await?;
    swap_community_image(&state, user, community_id, kind, Some((id, image.content_type))).await
}

async fn delete_community_image(
    state: AppState,
    user: AuthUser,
    community_id: Uuid,
    kind: CommunityImage,
) -> ApiResult<Json<Community>> {
    let ctx = permissions::member_context(&state.db, user.user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)?;
    swap_community_image(&state, user, community_id, kind, None).await
}

async fn get_community_image(
    state: AppState,
    user: AuthUser,
    community_id: Uuid,
    image_id: Uuid,
    kind: CommunityImage,
) -> ApiResult<Response> {
    permissions::member_context(&state.db, user.user_id, community_id).await?;
    let content_type: Option<Option<String>> = sqlx::query_scalar(kind.content_type_sql())
        .bind(community_id)
        .bind(image_id)
        .fetch_optional(&state.db)
        .await?;
    serve(&state, image_id, content_type.flatten(), CACHE_PRIVATE).await
}

async fn get_invite_image(
    state: AppState,
    code: String,
    image_id: Uuid,
    kind: CommunityImage,
) -> ApiResult<Response> {
    let content_type: Option<Option<String>> =
        sqlx::query_scalar(kind.invite_sql()).bind(&code).bind(image_id).fetch_optional(&state.db).await?;
    serve(&state, image_id, content_type.flatten(), CACHE_INVITE).await
}

/// `POST /communities/{id}/icon` – multipart, Feld `file`.
pub async fn upload_icon(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<Json<Community>> {
    upload_community_image(state, user, community_id, CommunityImage::Icon, multipart).await
}

/// `DELETE /communities/{id}/icon`
pub async fn delete_icon(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Community>> {
    delete_community_image(state, user, community_id, CommunityImage::Icon).await
}

/// `POST /communities/{id}/banner` – multipart, Feld `file`.
pub async fn upload_banner(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<Json<Community>> {
    upload_community_image(state, user, community_id, CommunityImage::Banner, multipart).await
}

/// `DELETE /communities/{id}/banner`
pub async fn delete_banner(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Community>> {
    delete_community_image(state, user, community_id, CommunityImage::Banner).await
}

/// `GET /communities/{id}/icon/{icon_id}` – nur Mitglieder.
pub async fn icon(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, icon_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    get_community_image(state, user, community_id, icon_id, CommunityImage::Icon).await
}

/// `GET /communities/{id}/banner/{banner_id}` – nur Mitglieder.
pub async fn banner(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, banner_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    get_community_image(state, user, community_id, banner_id, CommunityImage::Banner).await
}

/// `GET /invites/{code}/icon/{icon_id}` – öffentlich mit gültiger Einladung.
pub async fn invite_icon(
    State(state): State<AppState>,
    Path((code, icon_id)): Path<(String, Uuid)>,
) -> ApiResult<Response> {
    get_invite_image(state, code, icon_id, CommunityImage::Icon).await
}

/// `GET /invites/{code}/banner/{banner_id}` – öffentlich mit gültiger Einladung.
pub async fn invite_banner(
    State(state): State<AppState>,
    Path((code, banner_id)): Path<(String, Uuid)>,
) -> ApiResult<Response> {
    get_invite_image(state, code, banner_id, CommunityImage::Banner).await
}

/// Bild-URL für die Einladungsseite (gleiche Bild-ID, aber über den Einladungscode abrufbar).
pub fn invite_image_url(code: &str, kind: &str, image_id: Option<Uuid>) -> Option<String> {
    image_id.map(|id| format!("/api/invites/{}/{kind}/{id}", attachments::percent_encode(code)))
}

// --- Profilbanner ---------------------------------------------------------------

/// Setzt (`Some`) oder entfernt (`None`) das eigene Profilbanner.
async fn swap_user_banner(
    state: &AppState,
    user_id: Uuid,
    image: Option<(Uuid, &'static str)>,
) -> ApiResult<User> {
    let swapped: Result<Option<Option<Uuid>>, sqlx::Error> = sqlx::query_scalar(
        "UPDATE users u SET banner_id = $2, banner_content_type = $3
         FROM users old WHERE u.id = $1 AND old.id = u.id
         RETURNING old.banner_id",
    )
    .bind(user_id)
    .bind(image.map(|i| i.0))
    .bind(image.map(|i| i.1))
    .fetch_optional(&state.db)
    .await;
    let new_file: Vec<Uuid> = image.map(|i| i.0).into_iter().collect();
    let old = match swapped {
        Ok(Some(old)) => old,
        Ok(None) => {
            attachments::delete_files(state.storage.as_ref(), &new_file).await;
            return Err(ApiError::Unauthorized("unauthorized"));
        }
        Err(e) => {
            attachments::delete_files(state.storage.as_ref(), &new_file).await;
            return Err(e.into());
        }
    };
    if let Some(old) = old {
        attachments::delete_files(state.storage.as_ref(), &[old]).await;
    }
    users::announce_user_update(state, user_id).await
}

/// `POST /auth/me/banner` – multipart, Feld `file` (Client schneidet auf 600×240 zu).
pub async fn upload_user_banner(
    State(state): State<AppState>,
    user: AuthUser,
    multipart: Multipart,
) -> ApiResult<Json<User>> {
    let image = read_image(multipart, BANNER_MAX_BYTES, BANNER_MAX_SIDE).await?;
    let id = store(&state, image.data).await?;
    Ok(Json(swap_user_banner(&state, user.user_id, Some((id, image.content_type))).await?))
}

/// `DELETE /auth/me/banner`
pub async fn delete_user_banner(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<User>> {
    Ok(Json(swap_user_banner(&state, user.user_id, None).await?))
}

/// `GET /users/{id}/banner/{banner_id}` – wie der Avatar nur mit Sichtbarkeit (`can_see_user`).
pub async fn user_banner(
    State(state): State<AppState>,
    user: AuthUser,
    Path((user_id, banner_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    if !models::can_see_user(&state.db, user.user_id, user_id).await? {
        return Err(ApiError::not_found());
    }
    let content_type: Option<Option<String>> =
        sqlx::query_scalar("SELECT banner_content_type FROM users WHERE id = $1 AND banner_id = $2")
            .bind(user_id)
            .bind(banner_id)
            .fetch_optional(&state.db)
            .await?;
    serve(&state, banner_id, content_type.flatten(), CACHE_PRIVATE).await
}

// --- Systemnachrichten ------------------------------------------------------------

/// Schreibt „X ist beigetreten“ in den Systemkanal der Community (falls gesetzt) und verteilt sie.
/// Systemnachrichten haben `kind = member_join`, leeren Inhalt und den neuen Nutzer als Autor.
pub async fn post_join_message(state: &AppState, community_id: Uuid, user_id: Uuid) -> ApiResult<()> {
    let channel_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT ch.id FROM communities c JOIN channels ch ON ch.id = c.system_channel_id
         WHERE c.id = $1 AND ch.community_id = c.id AND ch.kind = 'text'",
    )
    .bind(community_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(channel_id) = channel_id else { return Ok(()) };
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, kind) VALUES ($1, $2, $3, '', 'member_join')",
    )
    .bind(id)
    .bind(channel_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    messages::broadcast(state, "MESSAGE_CREATE", id).await
}
