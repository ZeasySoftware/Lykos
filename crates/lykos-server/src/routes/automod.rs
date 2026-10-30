//! Verwaltung der AutoMod-Regeln einer Community (`MANAGE_COMMUNITY`). Die Prüfung selbst steckt in
//! `crate::automod`.

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    audit,
    auth::AuthUser,
    automod::{
        self, Actions, KeywordConfig, LinksConfig, MAX_CUSTOM_MESSAGE_CHARS, MAX_EXEMPTIONS, MentionsConfig,
        Rule, RuleConfig, RuleKind, SpamConfig,
        filters::{self, MAX_DOMAINS, MAX_TERMS},
    },
    error::{ApiError, ApiResult},
    permissions::{self, Permissions},
    state::AppState,
};

async fn require_manager(state: &AppState, user_id: Uuid, community_id: Uuid) -> ApiResult<()> {
    let ctx = permissions::member_context(&state.db, user_id, community_id).await?;
    permissions::require(ctx.base(), Permissions::MANAGE_COMMUNITY)
}

/// `GET /communities/{id}/automod` – alle vier Regeln (fehlende mit Voreinstellungen, alles aus).
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Rule>>> {
    require_manager(&state, user.user_id, community_id).await?;
    Ok(Json(automod::load_rules(&state.db, community_id).await?))
}

#[derive(Deserialize)]
pub struct RuleInput {
    kind: RuleKind,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    actions: Actions,
    #[serde(default)]
    alert_channel_id: Option<Uuid>,
    #[serde(default)]
    custom_message: Option<String>,
    #[serde(default)]
    exempt_role_ids: Vec<Uuid>,
    #[serde(default)]
    exempt_channel_ids: Vec<Uuid>,
    #[serde(default)]
    config: Value,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    rules: Vec<RuleInput>,
}

/// Liste bereinigen: über `clean` normalisieren, Duplikate entfernen, höchstens `max` Einträge.
fn clean_list(
    items: &[String],
    max: usize,
    error: &'static str,
    clean: impl Fn(&str) -> Option<String>,
) -> ApiResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items.iter().filter(|i| !i.trim().is_empty()) {
        let cleaned = clean(item).ok_or(ApiError::BadRequest(error))?;
        if seen.insert(filters::normalize(&cleaned)) {
            out.push(cleaned);
        }
    }
    if out.len() > max {
        return Err(ApiError::BadRequest(error));
    }
    Ok(out)
}

/// Art-spezifische Einstellungen prüfen.
fn validate_config(kind: RuleKind, value: Value) -> ApiResult<RuleConfig> {
    let value = if value.is_null() { json!({}) } else { value };
    let invalid = |_| ApiError::BadRequest("invalid_request");
    Ok(match kind {
        RuleKind::Keyword => {
            let c: KeywordConfig = serde_json::from_value(value).map_err(invalid)?;
            RuleConfig::Keyword(KeywordConfig {
                terms: clean_list(&c.terms, MAX_TERMS, "invalid_keyword", filters::clean_term)?,
                // Ausnahmen sind ganze Wörter ohne Platzhalter.
                allow: clean_list(&c.allow, MAX_TERMS, "invalid_keyword", |a| {
                    filters::clean_term(a).filter(|t| !t.contains('*'))
                })?,
            })
        }
        RuleKind::Links => {
            let c: LinksConfig = serde_json::from_value(value).map_err(invalid)?;
            RuleConfig::Links(LinksConfig {
                allowed_domains: clean_list(
                    &c.allowed_domains,
                    MAX_DOMAINS,
                    "invalid_domain",
                    filters::clean_domain,
                )?,
                block_foreign_invites: c.block_foreign_invites,
            })
        }
        RuleKind::Mentions => {
            let c: MentionsConfig = serde_json::from_value(value).map_err(invalid)?;
            if !(1..=50).contains(&c.max_mentions) {
                return Err(ApiError::BadRequest("invalid_limit"));
            }
            RuleConfig::Mentions(c)
        }
        RuleKind::Spam => {
            let c: SpamConfig = serde_json::from_value(value).map_err(invalid)?;
            let rate_ok = c.max_messages == 0
                || ((2..=50).contains(&c.max_messages) && (1..=60).contains(&c.per_seconds));
            let dup_ok = c.max_duplicates == 0 || (2..=20).contains(&c.max_duplicates);
            if !rate_ok || !dup_ok {
                return Err(ApiError::BadRequest("invalid_limit"));
            }
            // `per_seconds` bleibt auch bei abgeschalteter Rate gültig, damit der Client ihn wieder anzeigen kann.
            RuleConfig::Spam(SpamConfig { per_seconds: c.per_seconds.clamp(1, 60), ..c })
        }
    })
}

/// Eine Regel prüfen und bereinigen. Ausnahmen werden auf existierende Rollen/Kanäle der Community gekürzt.
async fn validate(state: &AppState, community_id: Uuid, input: RuleInput) -> ApiResult<Rule> {
    let config = validate_config(input.kind, input.config)?;
    let custom_message = input.custom_message.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    if custom_message.as_ref().is_some_and(|m| m.chars().count() > MAX_CUSTOM_MESSAGE_CHARS) {
        return Err(ApiError::BadRequest("invalid_message"));
    }
    if input.exempt_role_ids.len() > MAX_EXEMPTIONS || input.exempt_channel_ids.len() > MAX_EXEMPTIONS {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    if let Some(channel_id) = input.alert_channel_id {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM channels WHERE id = $1 AND community_id = $2 AND kind = 'text')",
        )
        .bind(channel_id)
        .bind(community_id)
        .fetch_one(&state.db)
        .await?;
        if !ok {
            return Err(ApiError::BadRequest("invalid_channel"));
        }
    }
    if input.enabled && input.actions.alert && input.alert_channel_id.is_none() {
        return Err(ApiError::BadRequest("alert_channel_required"));
    }
    if input.enabled && !(input.actions.block || input.actions.alert || input.actions.audit) {
        return Err(ApiError::BadRequest("invalid_actions"));
    }
    let exempt_role_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM roles WHERE community_id = $1 AND id = ANY($2) ORDER BY position DESC, id",
    )
    .bind(community_id)
    .bind(&input.exempt_role_ids)
    .fetch_all(&state.db)
    .await?;
    let exempt_channel_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE community_id = $1 AND id = ANY($2) ORDER BY position, id",
    )
    .bind(community_id)
    .bind(&input.exempt_channel_ids)
    .fetch_all(&state.db)
    .await?;
    Ok(Rule {
        kind: input.kind,
        enabled: input.enabled,
        actions: input.actions,
        alert_channel_id: input.alert_channel_id,
        custom_message,
        exempt_role_ids,
        exempt_channel_ids,
        config,
    })
}

/// `PUT /communities/{id}/automod` – Regeln speichern (nur die übergebenen Arten); liefert alle vier zurück.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Vec<Rule>>> {
    require_manager(&state, user.user_id, community_id).await?;
    let mut kinds = HashSet::new();
    if body.rules.iter().any(|r| !kinds.insert(r.kind)) {
        return Err(ApiError::BadRequest("invalid_request"));
    }
    let mut rules = Vec::with_capacity(body.rules.len());
    for input in body.rules {
        rules.push(validate(&state, community_id, input).await?);
    }

    let mut tx = state.db.begin().await?;
    for rule in &rules {
        sqlx::query(
            "INSERT INTO automod_rules (community_id, kind, enabled, config, action_block, action_alert,
                                        action_audit, alert_channel_id, custom_message, exempt_role_ids,
                                        exempt_channel_ids, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
             ON CONFLICT (community_id, kind) DO UPDATE SET
                enabled = EXCLUDED.enabled, config = EXCLUDED.config, action_block = EXCLUDED.action_block,
                action_alert = EXCLUDED.action_alert, action_audit = EXCLUDED.action_audit,
                alert_channel_id = EXCLUDED.alert_channel_id, custom_message = EXCLUDED.custom_message,
                exempt_role_ids = EXCLUDED.exempt_role_ids, exempt_channel_ids = EXCLUDED.exempt_channel_ids,
                updated_at = now()",
        )
        .bind(community_id)
        .bind(rule.kind.as_str())
        .bind(rule.enabled)
        .bind(rule.config_json())
        .bind(rule.actions.block)
        .bind(rule.actions.alert)
        .bind(rule.actions.audit)
        .bind(rule.alert_channel_id)
        .bind(&rule.custom_message)
        .bind(&rule.exempt_role_ids)
        .bind(&rule.exempt_channel_ids)
        .execute(&mut *tx)
        .await?;
    }
    if !rules.is_empty() {
        audit::record(
            &mut *tx,
            community_id,
            user.user_id,
            "automod.update",
            None,
            json!({
                "rules": rules.iter().map(|r| json!({ "kind": r.kind.as_str(), "enabled": r.enabled })).collect::<Vec<_>>(),
            }),
        )
        .await?;
    }
    tx.commit().await?;
    state.automod.invalidate(community_id);

    Ok(Json(automod::load_rules(&state.db, community_id).await?))
}
