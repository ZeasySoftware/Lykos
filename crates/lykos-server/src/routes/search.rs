//! Nachrichtensuche (Postgres-Volltextsuche).
//!
//! Gesucht wird gleichzeitig mit deutscher Wortstamm-Erkennung (`german`) und ohne (`simple`), jeweils
//! als Präfixsuche: „haus“ findet „Häuser“ und „Hausaufgabe“, Stoppwörter und Namen findet `simple`.
//! Ergebnisse kommen nur aus Kanälen, die der Nutzer sehen darf (bzw. aus eigenen DMs).

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult},
    models::{self, Message},
    permissions,
    state::AppState,
};

const MAX_QUERY_CHARS: usize = 200;
const MAX_TERMS: usize = 10;
const DEFAULT_LIMIT: i64 = 25;
const MAX_LIMIT: i64 = 50;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    author_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    /// Nur Treffer älter als diese Nachricht (Blättern).
    before: Option<Uuid>,
    limit: Option<i64>,
}

impl SearchQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

/// Baut aus der Eingabe eine `tsquery`: Wörter (Buchstaben/Ziffern) als Präfixe, UND-verknüpft.
/// Alle Sonderzeichen werden zu Trennern – damit kann die Eingabe die tsquery-Syntax nicht verändern.
pub fn build_tsquery(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(MAX_TERMS)
        .map(|t| format!("{}:*", t.to_lowercase()))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" & ")) }
}

fn parse(q: &SearchQuery) -> ApiResult<String> {
    if q.q.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    build_tsquery(&q.q).ok_or(ApiError::BadRequest("invalid_request"))
}

/// Kanalauswahl auf `channel_id` einschränken (ohne zu verraten, ob ein unsichtbarer Kanal existiert).
fn restrict(mut channels: Vec<Uuid>, only: Option<Uuid>) -> Vec<Uuid> {
    if let Some(only) = only {
        channels.retain(|id| *id == only);
    }
    channels
}

/// `GET /communities/{id}/search?q=&author_id=&channel_id=&before=&limit=` – neueste Treffer zuerst.
pub async fn community(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let tsquery = parse(&q)?;
    let visible = permissions::visible_channel_ids(&state.db, user.user_id, community_id).await?;
    let channels = restrict(visible, q.channel_id);
    let hits = models::search_messages(
        &state.db,
        &channels,
        &tsquery,
        q.author_id,
        q.before,
        q.limit(),
        user.user_id,
    )
    .await?;
    Ok(Json(hits))
}

/// `GET /dms/search?q=&author_id=&channel_id=&before=&limit=` – Suche über alle eigenen DMs.
pub async fn dms(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let tsquery = parse(&q)?;
    let own: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM dm_recipients WHERE user_id = $1")
        .bind(user.user_id)
        .fetch_all(&state.db)
        .await?;
    let channels = restrict(own, q.channel_id);
    let hits = models::search_messages(
        &state.db,
        &channels,
        &tsquery,
        q.author_id,
        q.before,
        q.limit(),
        user.user_id,
    )
    .await?;
    Ok(Json(hits))
}

#[cfg(test)]
mod tests {
    use super::build_tsquery;

    #[test]
    fn tsquery_is_sanitized() {
        assert_eq!(build_tsquery("Hallo Welt").as_deref(), Some("hallo:* & welt:*"));
        assert_eq!(build_tsquery("e-mail").as_deref(), Some("e:* & mail:*"));
        assert_eq!(build_tsquery("Häuser!").as_deref(), Some("häuser:*"));
        assert_eq!(build_tsquery("a' | !b & (c):*").as_deref(), Some("a:* & b:* & c:*"));
        assert_eq!(build_tsquery("  !!! "), None);
        assert_eq!(build_tsquery(&"w ".repeat(50)).unwrap().matches('&').count(), 9);
    }
}
