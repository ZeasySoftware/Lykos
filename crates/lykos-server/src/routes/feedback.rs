//! Fehler- und Vorschlagsmeldungen (Paket K): Melden, eigene Meldungen, Vorschläge der Community mit
//! Stimmen und die Verwaltung durch Instanz-Admins (Status, Antworten, Löschen, Gitea-Issue).
//! Datenobjekte und Prüfungen stehen in `crate::feedback`.

use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    admin::{self, InstanceAdmin},
    attachments, audit,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    events::Scope,
    feedback::{self, Feedback, Row, View},
    feedback_select,
    routes::attachments::multipart_error,
    state::AppState,
};

// --- Admin-Guard ----------------------------------------------------------------
//
// Verwaltung über den zentralen Guard `admin::InstanceAdmin` (Paket H); Admin-Aktionen landen im
// instanzweiten Audit-Log (ohne Meldungsinhalte).

// --- Laden ------------------------------------------------------------------------

async fn row(state: &AppState, viewer: Uuid, id: Uuid) -> ApiResult<Option<Row>> {
    Ok(sqlx::query_as(concat!(feedback_select!(), " WHERE f.id = $2"))
        .bind(viewer)
        .bind(id)
        .fetch_optional(&state.db)
        .await?)
}

/// Meldungen in der Ansicht `view` für `viewer`, mit Antworten.
async fn assemble(state: &AppState, viewer: Uuid, rows: Vec<(Row, View)>) -> ApiResult<Vec<Feedback>> {
    let ids: Vec<Uuid> = rows.iter().map(|(r, _)| r.id).collect();
    let include_internal = rows.iter().any(|(_, v)| *v == View::Admin);
    let mut replies: HashMap<Uuid, Vec<feedback::FeedbackReply>> = HashMap::new();
    for (feedback_id, reply) in feedback::replies_for(&state.db, &ids, include_internal).await? {
        replies.entry(feedback_id).or_default().push(reply);
    }
    Ok(rows
        .into_iter()
        .map(|(r, view)| {
            let replies = replies.remove(&r.id).unwrap_or_default();
            r.into_feedback(viewer, view, replies)
        })
        .collect())
}

async fn one(state: &AppState, viewer: Uuid, row: Row, view: View) -> ApiResult<Feedback> {
    let mut list = assemble(state, viewer, vec![(row, view)]).await?;
    list.pop().ok_or_else(ApiError::not_found)
}

/// Meldung, die `user` sehen darf, samt Ansicht; sonst `404`.
async fn visible(state: &AppState, user: &AuthUser, id: Uuid) -> ApiResult<(Row, View)> {
    let row = row(state, user.user_id, id).await?.ok_or_else(ApiError::not_found)?;
    let admin = admin::is_instance_admin(&state.db, user.user_id).await?;
    let view = row.view_for(user.user_id, admin).ok_or_else(ApiError::not_found)?;
    Ok((row, view))
}

/// `FEEDBACK_UPDATE` an den Melder (in seiner Ansicht).
async fn notify_reporter(state: &AppState, id: Uuid) -> ApiResult<()> {
    let Some(reporter) =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT reporter_id FROM feedback WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .flatten()
    else {
        return Ok(());
    };
    if let Some(row) = row(state, reporter, id).await? {
        let feedback = one(state, reporter, row, View::Reporter).await?;
        state.events.publish(Scope::User(reporter), "FEEDBACK_UPDATE", &feedback);
    }
    Ok(())
}

fn page_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 100)
}

// --- Melden -------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateFeedback {
    kind: String,
    title: String,
    description: String,
    /// Technische Infos (nur mit Opt-in); `null`/fehlend = keine.
    #[serde(default)]
    tech_info: Option<Value>,
}

/// `POST /feedback` – Meldung absenden. Höchstens `PER_HOUR` Meldungen pro Nutzer und Stunde.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateFeedback>,
) -> ApiResult<(StatusCode, Json<Feedback>)> {
    feedback::validate_kind(&body.kind)?;
    let title = feedback::clean_title(&body.title)?;
    let description =
        feedback::clean_text(&body.description, 1, feedback::DESCRIPTION_MAX_CHARS, "invalid_description")?;
    let tech_info = match body.tech_info {
        None | Some(Value::Null) => None,
        Some(raw) => Some(feedback::sanitize_tech_info(&raw)?),
    };

    // Rate-Limit über die Datenbank: überlebt Neustarts und gilt auch bei mehreren Instanzen.
    let (count, oldest): (i64, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT count(*), min(created_at) FROM feedback
         WHERE reporter_id = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(user.user_id)
    .fetch_one(&state.db)
    .await?;
    if count >= feedback::PER_HOUR {
        let free_at = oldest.map(|t| t + time::Duration::hours(1));
        let wait = free_at.map_or(3600, |t| (t - OffsetDateTime::now_utc()).whole_seconds());
        return Err(ApiError::RateLimited { retry_after: wait.clamp(1, 3600) as u64 });
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO feedback (id, reporter_id, kind, title, description, tech_info)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user.user_id)
    .bind(&body.kind)
    .bind(&title)
    .bind(&description)
    .bind(tech_info)
    .execute(&state.db)
    .await?;

    let row = row(&state, user.user_id, id).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(one(&state, user.user_id, row, View::Reporter).await?)))
}

/// `POST /feedback/{id}/screenshot` – multipart mit genau einem Feld `file` (PNG, JPEG, WebP oder GIF).
/// Nur der Melder, nur einmal und nur in der ersten Stunde nach dem Absenden.
pub async fn upload_screenshot(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> ApiResult<Json<Feedback>> {
    let row = row(&state, user.user_id, id).await?.filter(|r| r.reporter_id == Some(user.user_id));
    let row = row.ok_or_else(ApiError::not_found)?;
    if row.screenshot_id.is_some() {
        return Err(ApiError::Conflict("screenshot_exists"));
    }
    if (OffsetDateTime::now_utc() - row.created_at).whole_seconds() > feedback::SCREENSHOT_WINDOW_SECS {
        return Err(ApiError::forbidden());
    }

    let mut field = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("file") => f,
        Ok(_) => return Err(ApiError::BadRequest("invalid_request")),
        Err(e) => return Err(multipart_error(e)),
    };
    let mut data = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        data.extend_from_slice(&chunk);
        if data.len() > feedback::SCREENSHOT_MAX_BYTES {
            return Err(ApiError::PayloadTooLarge("file_too_large"));
        }
    }
    let content_type = attachments::detect_content_type(&data);
    let side = 1..=feedback::SCREENSHOT_MAX_SIDE;
    let valid = attachments::image_dimensions(content_type, &data)
        .is_some_and(|(w, h)| side.contains(&w) && side.contains(&h));
    if !valid {
        return Err(ApiError::BadRequest("invalid_image"));
    }

    let screenshot_id = Uuid::now_v7();
    let body = Bytes::from(data);
    state
        .storage
        .put(&screenshot_id.to_string(), Box::pin(stream::once(async move { Ok(body) })))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Screenshot speichern")))?;

    // Erst eintragen, wenn noch keiner gesetzt ist (parallele Uploads).
    let updated = sqlx::query(
        "UPDATE feedback SET screenshot_id = $2, screenshot_content_type = $3, updated_at = now()
         WHERE id = $1 AND screenshot_id IS NULL",
    )
    .bind(id)
    .bind(screenshot_id)
    .bind(content_type)
    .execute(&state.db)
    .await;
    match updated {
        Ok(r) if r.rows_affected() == 1 => {}
        Ok(_) => {
            attachments::delete_files(state.storage.as_ref(), &[screenshot_id]).await;
            return Err(ApiError::Conflict("screenshot_exists"));
        }
        Err(e) => {
            attachments::delete_files(state.storage.as_ref(), &[screenshot_id]).await;
            return Err(e.into());
        }
    }

    let row = self::row(&state, user.user_id, id).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(one(&state, user.user_id, row, View::Reporter).await?))
}

/// `GET /feedback/{id}/screenshot` – nur Melder und Admins.
pub async fn screenshot(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    let (row, view) = visible(&state, &user, id).await?;
    if view == View::Public {
        return Err(ApiError::not_found());
    }
    let (Some(key), Some(content_type)) = (row.screenshot_id, row.screenshot_content_type) else {
        return Err(ApiError::not_found());
    };
    if !attachments::INLINE_IMAGE_TYPES.contains(&content_type.as_str()) {
        return Err(ApiError::not_found());
    }
    let stream = state
        .storage
        .get(&key.to_string(), None)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("Screenshot lesen")))?
        .ok_or_else(ApiError::not_found)?;

    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    Ok(res)
}

// --- Eigene Meldungen & Vorschläge ---------------------------------------------------

#[derive(Deserialize)]
pub struct Page {
    before: Option<Uuid>,
    limit: Option<i64>,
}

/// `GET /feedback/mine?before=&limit=` – eigene Meldungen, neueste zuerst.
pub async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Feedback>>> {
    let rows: Vec<Row> = sqlx::query_as(concat!(
        feedback_select!(),
        " WHERE f.reporter_id = $1 AND ($2::uuid IS NULL OR f.id < $2) ORDER BY f.id DESC LIMIT $3"
    ))
    .bind(user.user_id)
    .bind(page.before)
    .bind(page_limit(page.limit))
    .fetch_all(&state.db)
    .await?;
    let rows = rows.into_iter().map(|r| (r, View::Reporter)).collect();
    Ok(Json(assemble(&state, user.user_id, rows).await?))
}

/// Alle eigenen Meldungen in Melder-Sicht (für den Datenexport, `GET /auth/me/export`).
pub async fn all_of_reporter(state: &AppState, user_id: Uuid) -> ApiResult<Vec<Feedback>> {
    let rows: Vec<Row> =
        sqlx::query_as(concat!(feedback_select!(), " WHERE f.reporter_id = $1 ORDER BY f.id"))
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
    let rows = rows.into_iter().map(|r| (r, View::Reporter)).collect();
    assemble(state, user_id, rows).await
}

#[derive(Deserialize)]
pub struct SuggestionQuery {
    /// `votes` (Standard) oder `new`.
    sort: Option<String>,
    status: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

/// `GET /feedback/suggestions?sort=votes|new&status=&offset=&limit=` – Vorschläge der Community.
pub async fn suggestions(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<SuggestionQuery>,
) -> ApiResult<Json<Vec<Feedback>>> {
    let by_votes = match q.sort.as_deref() {
        None | Some("votes") => true,
        Some("new") => false,
        Some(_) => return Err(ApiError::BadRequest("invalid_request")),
    };
    if let Some(status) = &q.status {
        feedback::validate_status(status)?;
    }
    let sql = if by_votes {
        concat!(
            feedback_select!(),
            " WHERE f.kind = 'suggestion' AND ($2::text IS NULL OR f.status = $2)",
            " ORDER BY vote_count DESC, f.id DESC LIMIT $3 OFFSET $4"
        )
    } else {
        concat!(
            feedback_select!(),
            " WHERE f.kind = 'suggestion' AND ($2::text IS NULL OR f.status = $2)",
            " ORDER BY f.id DESC LIMIT $3 OFFSET $4"
        )
    };
    let rows: Vec<Row> = sqlx::query_as(sql)
        .bind(user.user_id)
        .bind(q.status)
        .bind(page_limit(q.limit))
        .bind(q.offset.unwrap_or(0).clamp(0, 10_000))
        .fetch_all(&state.db)
        .await?;
    // Auch Admins sehen hier die Community-Ansicht; eigene Vorschläge erscheinen in der Melder-Ansicht.
    let rows = rows
        .into_iter()
        .map(|r| {
            let view = if r.reporter_id == Some(user.user_id) { View::Reporter } else { View::Public };
            (r, view)
        })
        .collect();
    Ok(Json(assemble(&state, user.user_id, rows).await?))
}

/// `GET /feedback/{id}` – eigene Meldung, Vorschlag oder (für Admins) jede Meldung.
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Feedback>> {
    let (row, view) = visible(&state, &user, id).await?;
    Ok(Json(one(&state, user.user_id, row, view).await?))
}

/// `DELETE /feedback/{id}` – eigene Meldung zurückziehen (samt Screenshot und Stimmen).
pub async fn delete_own(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let deleted: Option<Option<Uuid>> =
        sqlx::query_scalar("DELETE FROM feedback WHERE id = $1 AND reporter_id = $2 RETURNING screenshot_id")
            .bind(id)
            .bind(user.user_id)
            .fetch_optional(&state.db)
            .await?;
    let screenshot = deleted.ok_or_else(ApiError::not_found)?;
    if let Some(key) = screenshot {
        attachments::delete_files(state.storage.as_ref(), &[key]).await;
    }
    state.events.publish(Scope::User(user.user_id), "FEEDBACK_DELETE", json!({ "id": id }));
    Ok(StatusCode::NO_CONTENT)
}

/// Vorschlag, den `user` sehen darf (andere Typen: `400 not_a_suggestion`, fremde Fehlerberichte `404`).
async fn suggestion(state: &AppState, user: &AuthUser, id: Uuid) -> ApiResult<()> {
    let (row, _) = visible(state, user, id).await?;
    if row.kind != "suggestion" {
        return Err(ApiError::BadRequest("not_a_suggestion"));
    }
    Ok(())
}

async fn vote_state(state: &AppState, user: &AuthUser, id: Uuid) -> ApiResult<Json<Value>> {
    let (count, voted): (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM feedback_votes WHERE feedback_id = $1),
                EXISTS (SELECT 1 FROM feedback_votes WHERE feedback_id = $1 AND user_id = $2)",
    )
    .bind(id)
    .bind(user.user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({ "vote_count": count, "voted": voted })))
}

/// `PUT /feedback/{id}/vote` – Vorschlag unterstützen; zweimal → `409 already_voted`.
pub async fn vote(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    suggestion(&state, &user, id).await?;
    let inserted = sqlx::query(
        "INSERT INTO feedback_votes (feedback_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(user.user_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict("already_voted"));
    }
    vote_state(&state, &user, id).await
}

/// `DELETE /feedback/{id}/vote` – Stimme zurücknehmen (idempotent).
pub async fn unvote(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    suggestion(&state, &user, id).await?;
    sqlx::query("DELETE FROM feedback_votes WHERE feedback_id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.user_id)
        .execute(&state.db)
        .await?;
    vote_state(&state, &user, id).await
}

// --- Verwaltung (Instanz-Admins) -------------------------------------------------------

/// `GET /admin/feedback/settings` – was die Verwaltung anbieten kann.
pub async fn admin_settings(State(state): State<AppState>, _admin: InstanceAdmin) -> Json<Value> {
    Json(json!({ "gitea": state.config.feedback_gitea.is_some() }))
}

#[derive(Deserialize)]
pub struct AdminQuery {
    kind: Option<String>,
    status: Option<String>,
    before: Option<Uuid>,
    limit: Option<i64>,
}

/// `GET /admin/feedback?kind=&status=&before=&limit=` – alle Meldungen, neueste zuerst.
pub async fn admin_list(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Query(q): Query<AdminQuery>,
) -> ApiResult<Json<Vec<Feedback>>> {
    if let Some(kind) = &q.kind {
        feedback::validate_kind(kind)?;
    }
    if let Some(status) = &q.status {
        feedback::validate_status(status)?;
    }
    let rows: Vec<Row> = sqlx::query_as(concat!(
        feedback_select!(),
        " WHERE ($2::text IS NULL OR f.kind = $2) AND ($3::text IS NULL OR f.status = $3)
           AND ($4::uuid IS NULL OR f.id < $4)
         ORDER BY f.id DESC LIMIT $5"
    ))
    .bind(admin.user_id)
    .bind(q.kind)
    .bind(q.status)
    .bind(q.before)
    .bind(page_limit(q.limit))
    .fetch_all(&state.db)
    .await?;
    let rows = rows.into_iter().map(|r| (r, View::Admin)).collect();
    Ok(Json(assemble(&state, admin.user_id, rows).await?))
}

async fn admin_row(state: &AppState, admin: &AuthUser, id: Uuid) -> ApiResult<Row> {
    row(state, admin.user_id, id).await?.ok_or_else(ApiError::not_found)
}

/// `GET /admin/feedback/{id}` – Detail inkl. Melder, technischer Infos und interner Notizen.
pub async fn admin_get(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Feedback>> {
    let row = admin_row(&state, &admin, id).await?;
    Ok(Json(one(&state, admin.user_id, row, View::Admin).await?))
}

#[derive(Deserialize)]
pub struct UpdateFeedback {
    status: String,
}

/// `PATCH /admin/feedback/{id}` – Status setzen; der Melder bekommt `FEEDBACK_UPDATE`.
pub async fn admin_update(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateFeedback>,
) -> ApiResult<Json<Feedback>> {
    feedback::validate_status(&body.status)?;
    let changed =
        sqlx::query("UPDATE feedback SET status = $2, updated_at = now() WHERE id = $1 AND status <> $2")
            .bind(id)
            .bind(&body.status)
            .execute(&state.db)
            .await?
            .rows_affected();
    let row = admin_row(&state, &admin, id).await?;
    if changed > 0 {
        audit::record_instance(
            &state.db,
            admin.user_id,
            "feedback.status",
            Some(id),
            json!({ "status": body.status }),
        )
        .await?;
        notify_reporter(&state, id).await?;
    }
    Ok(Json(one(&state, admin.user_id, row, View::Admin).await?))
}

#[derive(Deserialize)]
pub struct CreateReply {
    body: String,
    /// Interne Notiz (nur für Admins sichtbar, kein Event an den Melder).
    #[serde(default)]
    internal: bool,
}

/// `POST /admin/feedback/{id}/replies` – Antwort an den Melder oder interne Notiz.
pub async fn admin_reply(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateReply>,
) -> ApiResult<(StatusCode, Json<Feedback>)> {
    let text = feedback::clean_text(&body.body, 1, feedback::REPLY_MAX_CHARS, "invalid_content")?;
    admin_row(&state, &admin, id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO feedback_replies (id, feedback_id, author_id, body, internal) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(admin.user_id)
    .bind(&text)
    .bind(body.internal)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE feedback SET updated_at = now() WHERE id = $1").bind(id).execute(&mut *tx).await?;
    audit::record_instance(
        &mut *tx,
        admin.user_id,
        "feedback.reply",
        Some(id),
        json!({ "internal": body.internal }),
    )
    .await?;
    tx.commit().await?;
    if !body.internal {
        notify_reporter(&state, id).await?;
    }
    let row = admin_row(&state, &admin, id).await?;
    Ok((StatusCode::CREATED, Json(one(&state, admin.user_id, row, View::Admin).await?)))
}

/// `DELETE /admin/feedback/{id}` – Meldung samt Screenshot, Stimmen und Antworten löschen.
pub async fn admin_delete(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let deleted: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("DELETE FROM feedback WHERE id = $1 RETURNING reporter_id, screenshot_id")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (reporter, screenshot) = deleted.ok_or_else(ApiError::not_found)?;
    audit::record_instance(&state.db, admin.user_id, "feedback.delete", Some(id), json!({})).await?;
    if let Some(key) = screenshot {
        attachments::delete_files(state.storage.as_ref(), &[key]).await;
    }
    if let Some(reporter) = reporter {
        state.events.publish(Scope::User(reporter), "FEEDBACK_DELETE", json!({ "id": id }));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/feedback/{id}/issue` – Meldung als Gitea-Issue anlegen (nur mit Konfiguration, nur einmal).
pub async fn admin_create_issue(
    State(state): State<AppState>,
    InstanceAdmin(admin): InstanceAdmin,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Feedback>> {
    let Some(gitea) = state.config.feedback_gitea.as_ref() else {
        return Err(ApiError::ServiceUnavailable("gitea_not_configured"));
    };
    let row = admin_row(&state, &admin, id).await?;
    if row.issue_url.is_some() {
        return Err(ApiError::Conflict("issue_exists"));
    }
    let (title, body) = feedback::issue_content(&row);
    let url = feedback::create_issue(gitea, &row.kind, &title, &body).await.map_err(|e| {
        tracing::warn!(error = %e, "Gitea-Issue konnte nicht angelegt werden");
        ApiError::ServiceUnavailable("gitea_unavailable")
    })?;
    sqlx::query("UPDATE feedback SET issue_url = $2, updated_at = now() WHERE id = $1 AND issue_url IS NULL")
        .bind(id)
        .bind(&url)
        .execute(&state.db)
        .await?;
    audit::record_instance(&state.db, admin.user_id, "feedback.issue", Some(id), json!({ "url": url }))
        .await?;
    let row = admin_row(&state, &admin, id).await?;
    Ok(Json(one(&state, admin.user_id, row, View::Admin).await?))
}
