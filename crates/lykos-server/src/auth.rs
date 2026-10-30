//! Passwörter, Session-Tokens und der `AuthUser`-Extractor.

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, header, request::Parts},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use sqlx::PgExecutor;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

pub const SESSION_COOKIE: &str = "lykos_session";
/// Präfix von Bot-Tokens – macht sie für Secret-Scanner (z. B. gitleaks, GitHub) erkennbar.
pub const BOT_TOKEN_PREFIX: &str = "lykb_";
/// Sessions laufen so lange nach der letzten Nutzung ab.
pub const SESSION_TTL: Duration = Duration::days(30);

// --- Passwörter -------------------------------------------------------------

/// Argon2id-Hash im PHC-Format. Läuft auf dem Blocking-Pool, weil bewusst teuer.
pub async fn hash_password(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("Passwort-Hashing fehlgeschlagen: {e}"))
    })
    .await
    .map_err(anyhow::Error::from)?
    .map_err(ApiError::from)
}

/// Prüft ein Passwort. Ohne Hash (unbekannter Nutzer) wird gegen einen Dummy-Hash geprüft,
/// damit die Antwortzeit nicht verrät, ob der Benutzername existiert.
pub async fn verify_password(password: String, hash: Option<String>) -> ApiResult<bool> {
    tokio::task::spawn_blocking(move || {
        let hash = hash.unwrap_or_else(|| DUMMY_HASH.get_or_init(dummy_hash).clone());
        Argon2::default().verify_password(password.as_bytes(), hash.as_str()).is_ok()
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))
}

static DUMMY_HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn dummy_hash() -> String {
    Argon2::default().hash_password(b"lykos-dummy-password").map(|h| h.to_string()).unwrap_or_default()
}

// --- Tokens & Sessions ------------------------------------------------------

/// Zufällige, URL-sichere Zeichenkette aus `bytes` Zufallsbytes.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    URL_SAFE_NO_PAD.encode(buf)
}

/// SHA-256 eines Tokens – gespeichert wird nur dieser Hash (Sessions, Reset-Links).
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Legt eine Session an und gibt das (nur hier im Klartext existierende) Token zurück.
/// `403 account_disabled`, wenn das Konto gesperrt ist (oder nicht existiert). Für Schritte vor der
/// Session (2FA-Challenge, E-Mail-Code), damit gesperrte Konten gar nicht erst so weit kommen.
pub async fn ensure_active(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<()> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND disabled_at IS NULL AND NOT is_bot)",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    if active { Ok(()) } else { Err(ApiError::Forbidden("account_disabled")) }
}

/// Gesperrte Konten (`users.disabled_at`) bekommen keine Session: `403 account_disabled`.
/// Jeder Anmeldeweg läuft hierüber – die Sperre gilt so automatisch für alle. Bot-Konten melden sich nie an
/// (sie nutzen nur ihr Token) und werden hier wie gesperrte behandelt.
pub async fn create_session(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    label: Option<String>,
) -> ApiResult<String> {
    let token = random_token(32);
    let label = label.map(|l| l.trim().chars().take(64).collect::<String>()).filter(|l| !l.is_empty());
    let inserted = sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, label, expires_at)
         SELECT $1, $2, $3, $4, $5 FROM users WHERE id = $2 AND disabled_at IS NULL AND NOT is_bot",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(token_hash(&token))
    .bind(label)
    .bind(OffsetDateTime::now_utc() + SESSION_TTL)
    .execute(db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Forbidden("account_disabled"));
    }
    Ok(token)
}

/// Löst ein Token zu (user_id, session_id) auf und verlängert die Session.
pub async fn resolve_token(state: &AppState, token: &str) -> ApiResult<Option<(Uuid, Uuid)>> {
    let row: Option<(Uuid, Uuid)> =
        // Sessions gesperrter Konten werden beim Sperren gelöscht; die Bedingung hier ist nur doppelter Boden.
        sqlx::query_as(
            "SELECT s.user_id, s.id FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now() AND u.disabled_at IS NULL",
        )
            .bind(token_hash(token))
            .fetch_optional(&state.db)
            .await?;

    if let Some((_, session_id)) = row {
        // Nur selten schreiben: höchstens alle 5 Minuten pro Session.
        sqlx::query(
            "UPDATE sessions SET last_used_at = now(), expires_at = now() + $2
             WHERE id = $1 AND last_used_at < now() - interval '5 minutes'",
        )
        .bind(session_id)
        .bind(SESSION_TTL)
        .execute(&state.db)
        .await?;
    }
    Ok(row)
}

/// Löst ein Bot-Token zu (user_id, token_id) auf. `token_id` wechselt mit jedem neuen Token und dient als
/// Session-ID (Gateway-Widerruf). Gesperrte Bots und – per Instanz-Einstellung `allow_bots = false` –
/// abgeschaltete Bots werden nicht angenommen.
pub async fn resolve_bot_token(state: &AppState, token: &str) -> ApiResult<Option<(Uuid, Uuid)>> {
    if !token.starts_with(BOT_TOKEN_PREFIX) {
        return Ok(None);
    }
    Ok(sqlx::query_as(
        "SELECT b.user_id, b.token_id FROM bots b JOIN users u ON u.id = b.user_id
         WHERE b.token_hash = $1 AND u.is_bot AND u.disabled_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM instance_settings WHERE key = 'allow_bots' AND value = 'false'::jsonb)",
    )
    .bind(token_hash(token))
    .fetch_optional(&state.db)
    .await?)
}

/// Zugangsdaten einer Anfrage. `Debug` zeigt das Token nicht (Tokens landen nie im Log).
pub enum Credential {
    /// Session-Token (Bearer oder Cookie).
    Session(String),
    /// Bot-Token (`Authorization: Bot …`).
    Bot(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Credential::Session(_) => "Credential::Session(…)",
            Credential::Bot(_) => "Credential::Bot(…)",
        })
    }
}

impl Credential {
    /// Token aus einem Gateway-`identify`: `Bot <token>` oder ein Token mit Bot-Präfix gilt als Bot-Token.
    pub fn from_identify(token: &str) -> Self {
        let token = token.trim();
        match token.strip_prefix("Bot ") {
            Some(bot) => Credential::Bot(bot.trim().to_owned()),
            None if token.starts_with(BOT_TOKEN_PREFIX) => Credential::Bot(token.to_owned()),
            None => Credential::Session(token.to_owned()),
        }
    }
}

/// Zugangsdaten aus `Authorization: Bot`, `Authorization: Bearer` oder dem Session-Cookie.
pub fn credential_from_headers(headers: &HeaderMap) -> Option<Credential> {
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bot ") {
            return Some(Credential::Bot(token.trim().to_owned()));
        }
        if let Some(token) = auth.strip_prefix("Bearer ") {
            return Some(Credential::Session(token.trim().to_owned()));
        }
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| Credential::Session(value.to_owned()))
}

/// Kommt die Anfrage mit einem Bot-Token (`Authorization: Bot …`)?
pub fn is_bot_request(headers: &HeaderMap) -> bool {
    matches!(credential_from_headers(headers), Some(Credential::Bot(_)))
}

/// Löst Zugangsdaten zum angemeldeten Nutzer bzw. Bot auf.
pub async fn resolve_credential(state: &AppState, credential: &Credential) -> ApiResult<Option<AuthUser>> {
    Ok(match credential {
        Credential::Session(token) => resolve_token(state, token)
            .await?
            .map(|(user_id, session_id)| AuthUser { user_id, session_id, is_bot: false }),
        Credential::Bot(token) => resolve_bot_token(state, token)
            .await?
            .map(|(user_id, session_id)| AuthUser { user_id, session_id, is_bot: true }),
    })
}

/// Token aus `Authorization: Bearer` oder dem Session-Cookie (keine Bot-Tokens).
pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    match credential_from_headers(headers)? {
        Credential::Session(token) => Some(token),
        Credential::Bot(_) => None,
    }
}

/// `Set-Cookie`-Wert für eine neue Session.
pub fn session_cookie(state: &AppState, token: &str) -> String {
    let secure = if state.config.public_url.starts_with("https://") { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; Path=/api; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        SESSION_TTL.whole_seconds()
    )
}

/// `Set-Cookie`-Wert, der das Session-Cookie löscht.
pub fn clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/api; HttpOnly; SameSite=Lax; Max-Age=0")
}

// --- Extractor --------------------------------------------------------------

/// Angemeldeter Nutzer oder Bot. Als Handler-Argument erzwingt er eine gültige Session bzw. ein gültiges
/// Bot-Token (sonst 401). Bots laufen durch dieselben Rechteprüfungen wie Menschen; Konto-, Sicherheits-,
/// Freundes- und ähnliche Endpunkte sperrt für sie zentral `routes::bots::reject_forbidden`.
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub user_id: Uuid,
    /// Session-ID; bei Bots die `token_id` des aktuellen Tokens.
    pub session_id: Uuid,
    pub is_bot: bool,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let credential =
            credential_from_headers(&parts.headers).ok_or(ApiError::Unauthorized("unauthorized"))?;
        resolve_credential(state, &credential).await?.ok_or(ApiError::Unauthorized("unauthorized"))
    }
}

/// Wie `AuthUser`, aber optional (für öffentliche Endpunkte mit Zusatzinfos für Angemeldete).
pub struct MaybeAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(Self(Some(user))),
            Err(ApiError::Unauthorized(_)) => Ok(Self(None)),
            Err(e) => Err(e),
        }
    }
}

// --- Validierung ------------------------------------------------------------

pub fn validate_username(username: &str) -> ApiResult<()> {
    let ok = (2..=32).contains(&username.chars().count())
        && username.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok { Ok(()) } else { Err(ApiError::BadRequest("invalid_username")) }
}

pub fn validate_password(password: &str) -> ApiResult<()> {
    if (10..=256).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(ApiError::BadRequest("invalid_password"))
    }
}

/// Anzeigename: getrimmt, max. 64 Zeichen; leer → `None`.
pub fn normalize_display_name(name: Option<String>) -> ApiResult<Option<String>> {
    let Some(name) = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().count() > 64 {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    Ok(Some(name))
}

/// Höchstlänge von „Über mich“ (Zeichen).
pub const BIO_MAX_CHARS: usize = 190;
/// Höchstlänge des Status-Texts (Zeichen).
pub const STATUS_TEXT_MAX_CHARS: usize = 128;
/// Höchstlänge eines Nicknames (Zeichen).
pub const NICKNAME_MAX_CHARS: usize = 32;

/// Freitext-Profilfeld: getrimmt, ohne Steuerzeichen außer Zeilenumbruch, höchstens `max` Zeichen;
/// leer → `None`. Zu lang → `BadRequest(code)`.
pub fn normalize_text(text: Option<String>, max: usize, code: &'static str) -> ApiResult<Option<String>> {
    let Some(text) = text else { return Ok(None) };
    let cleaned: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > max {
        return Err(ApiError::BadRequest(code));
    }
    Ok(Some(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_texts() {
        assert_eq!(normalize_text(None, 5, "x").unwrap(), None);
        assert_eq!(normalize_text(Some("  ".into()), 5, "x").unwrap(), None);
        assert_eq!(normalize_text(Some(" a\u{0}b\nc ".into()), 5, "x").unwrap().as_deref(), Some("ab\nc"));
        assert_eq!(normalize_text(Some("äöüßé".into()), 5, "x").unwrap().as_deref(), Some("äöüßé"));
        assert!(matches!(normalize_text(Some("abcdef".into()), 5, "x"), Err(ApiError::BadRequest("x"))));
    }

    #[test]
    fn usernames() {
        assert!(validate_username("alex").is_ok());
        assert!(validate_username("a.b-c_d").is_ok());
        assert!(validate_username("a").is_err());
        assert!(validate_username("mit leerzeichen").is_err());
        assert!(validate_username("ümlaut").is_err());
        assert!(validate_username(&"x".repeat(33)).is_err());
    }

    #[test]
    fn token_from_bearer_and_cookie() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "foo=bar; lykos_session=abc123".parse().unwrap());
        assert_eq!(token_from_headers(&h).as_deref(), Some("abc123"));
        h.insert(header::AUTHORIZATION, "Bearer xyz".parse().unwrap());
        assert_eq!(token_from_headers(&h).as_deref(), Some("xyz"));
        assert!(!is_bot_request(&h));
        h.insert(header::AUTHORIZATION, "Bot lykb_abc".parse().unwrap());
        assert!(is_bot_request(&h));
        assert_eq!(token_from_headers(&h), None);
        assert!(matches!(credential_from_headers(&h), Some(Credential::Bot(t)) if t == "lykb_abc"));
        assert_eq!(format!("{:?}", credential_from_headers(&h).unwrap()), "Credential::Bot(…)");
    }

    #[test]
    fn identify_tokens() {
        assert!(matches!(Credential::from_identify("Bot lykb_x"), Credential::Bot(t) if t == "lykb_x"));
        assert!(matches!(Credential::from_identify("lykb_x"), Credential::Bot(t) if t == "lykb_x"));
        assert!(matches!(Credential::from_identify("abc"), Credential::Session(t) if t == "abc"));
    }

    #[test]
    fn tokens_are_unique_and_urlsafe() {
        let a = random_token(32);
        let b = random_token(32);
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[tokio::test]
    async fn password_roundtrip() {
        let hash = hash_password("richtig-langes-passwort".into()).await.unwrap();
        assert!(verify_password("richtig-langes-passwort".into(), Some(hash.clone())).await.unwrap());
        assert!(!verify_password("falsch-falsch-falsch".into(), Some(hash)).await.unwrap());
        assert!(!verify_password("egal-was-hier-steht".into(), None).await.unwrap());
    }
}
