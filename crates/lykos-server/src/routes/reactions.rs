//! Emoji-Reaktionen auf Nachrichten – Unicode-Emojis und Custom-Emotes (`name:id`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    emojis,
    error::{ApiError, ApiResult},
    events, mentions,
    permissions::{self, Permissions},
    state::AppState,
};

/// Höchstzahl verschiedener Emojis pro Nachricht.
pub const MAX_EMOJIS_PER_MESSAGE: i64 = 20;

async fn channel_of(state: &AppState, message_id: Uuid) -> ApiResult<Uuid> {
    sqlx::query_scalar("SELECT channel_id FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}

/// Aufgelöste Reaktion: Schlüssel in `message_reactions.emoji` und Angaben für Events.
struct ReactionKey {
    /// Unicode-Emoji oder die Emote-ID als Text.
    key: String,
    /// So erscheint das Emoji in Events (`name:id` bei Custom-Emotes).
    display: String,
    emoji_id: Option<Uuid>,
    /// Name zum Speichern (`emoji_name`), nur bei Custom-Emotes.
    name: Option<String>,
    animated: bool,
    /// Community, der das Emote gehört (`None` = gelöscht oder Unicode).
    community_id: Option<Uuid>,
}

async fn resolve(state: &AppState, emoji: &str) -> ApiResult<ReactionKey> {
    if let Some((name, id)) = emojis::parse_reaction(emoji) {
        let found: Option<(String, bool, Uuid)> =
            sqlx::query_as("SELECT name, animated, community_id FROM custom_emojis WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await?;
        let (name, animated, community_id) = match found {
            Some((name, animated, community)) => (name, animated, Some(community)),
            None => (name.to_owned(), false, None),
        };
        return Ok(ReactionKey {
            key: id.to_string(),
            display: format!("{name}:{id}"),
            emoji_id: Some(id),
            name: Some(name),
            animated,
            community_id,
        });
    }
    if !mentions::is_valid_emoji(emoji) {
        return Err(ApiError::BadRequest("invalid_emoji"));
    }
    Ok(ReactionKey {
        key: emoji.to_owned(),
        display: emoji.to_owned(),
        emoji_id: None,
        name: None,
        animated: false,
        community_id: None,
    })
}

fn publish(
    state: &AppState,
    kind: &'static str,
    community_id: Option<Uuid>,
    channel_id: Uuid,
    message_id: Uuid,
    user_id: Uuid,
    reaction: &ReactionKey,
) {
    state.events.publish(
        events::channel(community_id, channel_id),
        kind,
        json!({
            "message_id": message_id,
            "channel_id": channel_id,
            "community_id": community_id,
            "user_id": user_id,
            "emoji": reaction.display,
            "emoji_id": reaction.emoji_id,
            "animated": reaction.animated,
        }),
    );
}

/// `PUT /messages/{id}/reactions/{emoji}/@me` – eigene Reaktion hinzufügen (idempotent). Antwort 204.
///
/// Custom-Emotes (`name:id`): Eine **neue** Reaktion darf nur setzen, wer Mitglied der Community des Emotes
/// ist (sonst `403`; unbekanntes Emote `invalid_emoji`). Einer bestehenden Reaktion darf sich jeder
/// anschließen, der reagieren darf.
pub async fn add(
    State(state): State<AppState>,
    user: AuthUser,
    Path((message_id, emoji)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let channel_id = channel_of(&state, message_id).await?;
    let (ctx, perms) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    permissions::require(perms, Permissions::ADD_REACTIONS)?;
    let reaction = resolve(&state, &emoji).await?;

    let (distinct, exists): (i64, bool) = sqlx::query_as(
        "SELECT count(DISTINCT emoji), COALESCE(bool_or(emoji = $2), false)
         FROM message_reactions WHERE message_id = $1",
    )
    .bind(message_id)
    .bind(&reaction.key)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        if reaction.emoji_id.is_some() {
            let Some(community_id) = reaction.community_id else {
                return Err(ApiError::BadRequest("invalid_emoji"));
            };
            let member: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM members WHERE community_id = $1 AND user_id = $2)",
            )
            .bind(community_id)
            .bind(user.user_id)
            .fetch_one(&state.db)
            .await?;
            if !member {
                return Err(ApiError::forbidden());
            }
        }
        if distinct >= MAX_EMOJIS_PER_MESSAGE {
            return Err(ApiError::BadRequest("too_many_reactions"));
        }
    }

    let added = sqlx::query(
        "INSERT INTO message_reactions (message_id, emoji, user_id, emoji_name) VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(message_id)
    .bind(&reaction.key)
    .bind(user.user_id)
    .bind(&reaction.name)
    .execute(&state.db)
    .await?
    .rows_affected();
    if added > 0 {
        publish(
            &state,
            "MESSAGE_REACTION_ADD",
            ctx.community_id,
            channel_id,
            message_id,
            user.user_id,
            &reaction,
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /messages/{id}/reactions/{emoji}/@me` – eigene Reaktion entfernen (idempotent). Antwort 204.
pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path((message_id, emoji)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let channel_id = channel_of(&state, message_id).await?;
    let (ctx, _) = permissions::channel_permissions(&state.db, user.user_id, channel_id).await?;
    let reaction = match emojis::parse_reaction(&emoji) {
        Some(_) => resolve(&state, &emoji).await?,
        // Unicode: ohne Prüfung entfernen (eine ungültige Reaktion existiert ohnehin nicht).
        None => ReactionKey {
            key: emoji.clone(),
            display: emoji.clone(),
            emoji_id: None,
            name: None,
            animated: false,
            community_id: None,
        },
    };

    let removed =
        sqlx::query("DELETE FROM message_reactions WHERE message_id = $1 AND emoji = $2 AND user_id = $3")
            .bind(message_id)
            .bind(&reaction.key)
            .bind(user.user_id)
            .execute(&state.db)
            .await?
            .rows_affected();
    if removed > 0 {
        publish(
            &state,
            "MESSAGE_REACTION_REMOVE",
            ctx.community_id,
            channel_id,
            message_id,
            user.user_id,
            &reaction,
        );
    }
    Ok(StatusCode::NO_CONTENT)
}
