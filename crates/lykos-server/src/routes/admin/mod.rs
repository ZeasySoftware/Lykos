//! Instanz-Verwaltung (Paket H): alle Routen unter `/admin/*`, nur für Instanz-Admins.
//!
//! Die Rechteprüfung steckt ausschließlich im Extractor [`InstanceAdmin`] – jeder Handler hier nimmt ihn
//! als Argument. Jede ändernde Aktion landet im instanzweiten Audit-Log (`audit::record_instance`).
//!
//! Datenschutz: Admins sehen keine Nachrichteninhalte und keine IP-Adressen (werden ohnehin nicht
//! gespeichert), nur Zählwerte und Verwaltungsdaten.

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post, put},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    admin::InstanceAdmin,
    audit::{self, AuditEntry},
    error::ApiResult,
    state::AppState,
};

mod communities;
mod invites;
mod overview;
mod settings;
mod users;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/overview", get(overview::overview))
        .route("/admin/invites", get(invites::list).post(invites::create))
        .route("/admin/invites/{code}", axum::routing::delete(invites::revoke))
        .route("/admin/users", get(users::list))
        .route("/admin/users/{id}", get(users::detail).delete(users::delete))
        .route("/admin/users/{id}/disable", post(users::disable))
        .route("/admin/users/{id}/enable", post(users::enable))
        .route("/admin/users/{id}/sessions", axum::routing::delete(users::revoke_sessions))
        .route("/admin/users/{id}/admin", put(users::grant_admin).delete(users::revoke_admin))
        .route("/admin/users/{id}/password-reset", post(users::password_reset_link))
        .route("/admin/communities", get(communities::list))
        .route("/admin/communities/{id}", axum::routing::delete(communities::delete))
        .route("/admin/communities/{id}/members", get(communities::members))
        .route("/admin/communities/{id}/transfer", post(communities::transfer))
        .route("/admin/settings", get(settings::get).patch(settings::update))
        .route("/admin/audit-log", get(audit_log))
}

/// Suche und Seitenaufteilung für Listen (`page` ab 0).
#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
    /// Nur Nutzerliste: `admins` oder `disabled`.
    filter: Option<String>,
}

impl ListQuery {
    fn limit(&self) -> i64 {
        self.per_page.unwrap_or(50).clamp(1, 100)
    }

    fn offset(&self) -> i64 {
        self.page.unwrap_or(0).clamp(0, 100_000) * self.limit()
    }

    /// `ILIKE`-Muster für die Suche (Platzhalter im Suchtext maskiert); `None` = keine Suche.
    fn pattern(&self) -> Option<String> {
        let q = self.q.as_deref().map(str::trim).filter(|q| !q.is_empty())?;
        let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        Some(format!("%{escaped}%"))
    }
}

/// Liste mit Gesamtzahl (für die Seitenanzeige).
#[derive(serde::Serialize)]
pub struct Page<T> {
    total: i64,
    items: Vec<T>,
}

#[derive(Deserialize)]
pub struct AuditQuery {
    before: Option<Uuid>,
    limit: Option<i64>,
}

/// `GET /admin/audit-log` – instanzweites Audit-Log, neueste zuerst.
async fn audit_log(
    State(state): State<AppState>,
    _admin: InstanceAdmin,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<AuditEntry>>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    Ok(Json(audit::instance_page(&state.db, q.before, limit).await?))
}

/// Öffentliche Basis-URL ohne abschließenden Slash (für Links).
fn base_url(state: &AppState) -> &str {
    state.config.public_url.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::ListQuery;

    #[test]
    fn search_pattern_escapes_wildcards() {
        let q = |s: &str| ListQuery { q: Some(s.into()), page: None, per_page: None, filter: None };
        assert_eq!(q("  ").pattern(), None);
        assert_eq!(q("alex").pattern().as_deref(), Some("%alex%"));
        assert_eq!(q("a_b%c\\").pattern().as_deref(), Some("%a\\_b\\%c\\\\%"));
        assert_eq!(ListQuery { q: None, page: Some(2), per_page: Some(500), filter: None }.offset(), 200);
    }
}
