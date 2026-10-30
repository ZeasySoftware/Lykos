//! Custom-Emotes einer Community: auflisten, hochladen, umbenennen, löschen und das Bild abrufen.
//!
//! Verwalten dürfen Besitzer, Mitglieder mit `MANAGE_EMOJIS` und Instanz-Admins. Wie bei Avataren dekodiert
//! der Server keine Bilder, sondern prüft Typ (Magic Bytes), Größe und Maße; der Client verkleinert statische
//! Bilder vorher per Canvas. Animierte GIFs werden nie neu kodiert (das würde die Animation zerstören).
//!
//! Datenschutz-Abwägung beim Abruf: Emote-Bilder sind für **alle angemeldeten Nutzer der Instanz** abrufbar.
//! Emotes dürfen überall verwendet werden (auch in DMs und fremden Communities); eine exakte Prüfung „steckt
//! in einer für dich sichtbaren Nachricht oder Reaktion“ wäre bei jedem Bildabruf eine teure Volltextsuche.
//! Die ID ist eine zufällige UUIDv7 und nur über Nachrichten/Reaktionen bekannt; Name und Community verrät
//! der Bildabruf nicht. Mitgliedern fremder Communities liefert der Server keine Emote-Liste dieser Community.

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use futures_util::stream;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    attachments, audit,
    auth::AuthUser,
    emojis::{self, CustomEmoji},
    error::{ApiError, ApiResult, is_unique_violation},
    events,
    permissions::{self, Permissions},
    routes::attachments::multipart_error,
    state::AppState,
};

/// Ist der Nutzer Instanz-Admin?
async fn is_instance_admin(state: &AppState, user_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar("SELECT is_instance_admin FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(false))
}

async fn community_exists(state: &AppState, community_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM communities WHERE id = $1)")
        .bind(community_id)
        .fetch_one(&state.db)
        .await?)
}

/// Lesen: Mitglied oder Instanz-Admin; sonst `404`.
async fn require_view(state: &AppState, user_id: Uuid, community_id: Uuid) -> ApiResult<()> {
    match permissions::member_context(&state.db, user_id, community_id).await {
        Ok(_) => Ok(()),
        Err(ApiError::NotFound(_))
            if is_instance_admin(state, user_id).await? && community_exists(state, community_id).await? =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Verwalten: `MANAGE_EMOJIS` (Besitzer/Administrator inklusive) oder Instanz-Admin.
/// Mitglieder ohne Recht → `403`, Nicht-Mitglieder → `404`.
async fn require_manage(state: &AppState, user_id: Uuid, community_id: Uuid) -> ApiResult<()> {
    let member = permissions::member_context(&state.db, user_id, community_id).await;
    if let Ok(ctx) = &member
        && ctx.base().contains(Permissions::MANAGE_EMOJIS)
    {
        return Ok(());
    }
    if is_instance_admin(state, user_id).await? && community_exists(state, community_id).await? {
        return Ok(());
    }
    match member {
        Ok(_) => Err(ApiError::forbidden()),
        Err(e) => Err(e),
    }
}

/// Aktuelle Liste an alle Mitglieder verteilen.
async fn announce(state: &AppState, community_id: Uuid) -> ApiResult<()> {
    let list = emojis::of_community(&state.db, community_id).await?;
    state.events.publish(
        events::community(community_id),
        "EMOJIS_UPDATE",
        json!({ "community_id": community_id, "emojis": list }),
    );
    Ok(())
}

/// `GET /emojis` – alle Emotes der eigenen Communities (die man überall verwenden darf).
pub async fn mine(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<CustomEmoji>>> {
    Ok(Json(emojis::usable_by(&state.db, user.user_id).await?))
}

/// `GET /communities/{id}/emojis` – Emotes einer Community.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<CustomEmoji>>> {
    require_view(&state, user.user_id, community_id).await?;
    Ok(Json(emojis::of_community(&state.db, community_id).await?))
}

/// `POST /communities/{id}/emojis` – multipart mit den Feldern `name` und `file`. Antwort 201.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<CustomEmoji>)> {
    require_manage(&state, user.user_id, community_id).await?;

    let mut name: Option<String> = None;
    let mut data: Option<Vec<u8>> = None;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("name") if name.is_none() => {
                name = Some(field.text().await.map_err(multipart_error)?.trim().to_owned());
            }
            Some("file") if data.is_none() => {
                let mut buf = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    buf.extend_from_slice(&chunk);
                    if buf.len() > emojis::MAX_BYTES {
                        return Err(ApiError::PayloadTooLarge("file_too_large"));
                    }
                }
                data = Some(buf);
            }
            _ => return Err(ApiError::BadRequest("invalid_request")),
        }
    }
    let (Some(name), Some(data)) = (name, data) else {
        return Err(ApiError::BadRequest("invalid_request"));
    };
    if !emojis::is_valid_name(&name) {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    let content_type = attachments::detect_content_type(&data);
    let side = 1..=emojis::MAX_SIDE;
    let valid = emojis::ALLOWED_TYPES.contains(&content_type)
        && attachments::image_dimensions(content_type, &data)
            .is_some_and(|(w, h)| side.contains(&w) && side.contains(&h));
    if !valid {
        return Err(ApiError::BadRequest("invalid_image"));
    }

    // Erst prüfen (gesperrte Community-Zeile serialisiert parallele Uploads), dann speichern, dann eintragen.
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
        .bind(community_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let (count, taken): (i64, bool) = sqlx::query_as(
        "SELECT count(*), COALESCE(bool_or(lower(name) = lower($2)), false)
         FROM custom_emojis WHERE community_id = $1",
    )
    .bind(community_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(ApiError::Conflict("emoji_name_taken"));
    }
    if count >= emojis::MAX_PER_COMMUNITY {
        return Err(ApiError::BadRequest("too_many_emojis"));
    }

    let id = Uuid::now_v7();
    let size = data.len() as i32;
    let body = Bytes::from(data);
    state
        .storage
        .put(&id.to_string(), Box::pin(stream::once(async move { Ok(body) })))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Emote speichern")))?;

    let result = async {
        let inserted = sqlx::query(
            "INSERT INTO custom_emojis (id, community_id, name, animated, content_type, size_bytes, uploader_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(community_id)
        .bind(&name)
        .bind(content_type == "image/gif")
        .bind(content_type)
        .bind(size)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await;
        match inserted {
            Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("emoji_name_taken")),
            other => other?,
        };
        audit::record(&mut *tx, community_id, user.user_id, "emoji.create", Some(id), json!({ "name": name }))
            .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        attachments::delete_files(state.storage.as_ref(), &[id]).await;
        return Err(e);
    }

    announce(&state, community_id).await?;
    let emoji = emojis::by_id(&state.db, id).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(emoji)))
}

#[derive(Deserialize)]
pub struct RenameBody {
    name: String,
}

/// Emote der Community laden; fremde IDs sind `404`.
async fn emoji_in(state: &AppState, community_id: Uuid, emoji_id: Uuid) -> ApiResult<CustomEmoji> {
    emojis::by_id(&state.db, emoji_id)
        .await?
        .filter(|e| e.community_id == community_id)
        .ok_or_else(ApiError::not_found)
}

/// `PATCH /communities/{id}/emojis/{emoji_id}` – umbenennen.
pub async fn rename(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, emoji_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<RenameBody>,
) -> ApiResult<Json<CustomEmoji>> {
    require_manage(&state, user.user_id, community_id).await?;
    let old = emoji_in(&state, community_id, emoji_id).await?;
    let name = body.name.trim();
    if !emojis::is_valid_name(name) {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    if name == old.name {
        return Ok(Json(old));
    }

    let mut tx = state.db.begin().await?;
    let updated = sqlx::query("UPDATE custom_emojis SET name = $2 WHERE id = $1")
        .bind(emoji_id)
        .bind(name)
        .execute(&mut *tx)
        .await;
    match updated {
        Err(e) if is_unique_violation(&e) => return Err(ApiError::Conflict("emoji_name_taken")),
        other => other?,
    };
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "emoji.update",
        Some(emoji_id),
        json!({ "name": name, "old_name": old.name }),
    )
    .await?;
    tx.commit().await?;

    announce(&state, community_id).await?;
    Ok(Json(emoji_in(&state, community_id, emoji_id).await?))
}

/// `DELETE /communities/{id}/emojis/{emoji_id}` – löschen. Nachrichten behalten ihr Token; Clients zeigen
/// dann den Platzhalter `:name:`.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path((community_id, emoji_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    require_manage(&state, user.user_id, community_id).await?;
    let emoji = emoji_in(&state, community_id, emoji_id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM custom_emojis WHERE id = $1").bind(emoji_id).execute(&mut *tx).await?;
    audit::record(
        &mut *tx,
        community_id,
        user.user_id,
        "emoji.delete",
        Some(emoji_id),
        json!({ "name": emoji.name }),
    )
    .await?;
    tx.commit().await?;
    attachments::delete_files(state.storage.as_ref(), &[emoji_id]).await;

    announce(&state, community_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /emojis/{id}.{ext}` – das Bild. Jeder angemeldete Nutzer (siehe Moduldoku); die Endung ist
/// optional und wird ignoriert, ausgeliefert wird der gespeicherte Typ.
pub async fn image(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(file): Path<String>,
) -> ApiResult<Response> {
    let id = file.split_once('.').map_or(file.as_str(), |(id, _)| id);
    let id = Uuid::try_parse(id).map_err(|_| ApiError::not_found())?;
    let content_type: String = sqlx::query_scalar("SELECT content_type FROM custom_emojis WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .filter(|t: &String| emojis::ALLOWED_TYPES.contains(&t.as_str()))
        .ok_or_else(ApiError::not_found)?;

    let stream = state
        .storage
        .get(&id.to_string(), None)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Emote lesen")))?
        .ok_or_else(ApiError::not_found)?;

    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // Emote-IDs ändern sich nie (Umbenennen ändert nur den Namen) → Caches dürfen ewig halten.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    *res.status_mut() = StatusCode::OK;
    Ok(res)
}
