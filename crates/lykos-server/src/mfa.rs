//! Zwei-Faktor-Authentifizierung: TOTP (RFC 6238), Wiederherstellungscodes, Codes per E-Mail, der zweite
//! Anmeldeschritt (`mfa_token`) und die Re-Authentifizierung vor sensiblen Aktionen.
//!
//! Gespeichert werden nur Hashes von Codes und Tokens. Das TOTP-Geheimnis liegt ausschließlich auf dem
//! Server (verlässt ihn nur einmal bei der Einrichtung) und taucht weder im Export noch in Logs auf.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::PgExecutor;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    auth::{self, AuthUser},
    error::{ApiError, ApiResult},
    mail,
    state::AppState,
};

/// Länge eines TOTP-Zeitschritts in Sekunden.
pub const TOTP_PERIOD: i64 = 30;
/// Akzeptierte Abweichung in Zeitschritten (±1 = ±30 s).
const TOTP_SKEW: i64 = 1;
/// Länge des TOTP-Geheimnisses in Bytes (160 Bit, wie von RFC 4226 empfohlen).
const TOTP_SECRET_BYTES: usize = 20;
/// Anzahl der Wiederherstellungscodes.
pub const RECOVERY_CODES: usize = 10;
/// E-Mail-Codes: Gültigkeit und Fehlversuche.
const EMAIL_CODE_TTL: Duration = Duration::minutes(10);
const EMAIL_CODE_MAX_ATTEMPTS: i32 = 5;
/// Zweiter Anmeldeschritt: Gültigkeit und Fehlversuche.
pub const CHALLENGE_TTL: Duration = Duration::minutes(5);
const CHALLENGE_MAX_ATTEMPTS: i32 = 5;
/// Konten ohne Passwort (nur OIDC) bestätigen sensible Aktionen über eine frische Anmeldung.
const RECENT_LOGIN: Duration = Duration::minutes(10);

// --- TOTP ---------------------------------------------------------------------------------------------

/// 6-stelliger TOTP-Code für einen Zeitschritt (HMAC-SHA1, dynamische Kürzung nach RFC 4226).
pub fn totp_at(secret: &[u8], step: i64) -> u32 {
    let mut mac =
        <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC akzeptiert jede Schlüssellänge");
    mac.update(&(step as u64).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([hash[offset] & 0x7f, hash[offset + 1], hash[offset + 2], hash[offset + 3]]);
    bin % 1_000_000
}

/// Aktueller Zeitschritt.
pub fn current_step() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp().div_euclid(TOTP_PERIOD)
}

/// Neues zufälliges TOTP-Geheimnis.
pub fn new_totp_secret() -> Vec<u8> {
    let mut buf = vec![0u8; TOTP_SECRET_BYTES];
    rand::fill(&mut buf[..]);
    buf
}

/// Base32 (RFC 4648, ohne Padding) – das Format, das Authenticator-Apps erwarten.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// `otpauth://`-URI für den QR-Code.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    let label = format!("{}:{}", uri_encode(issuer), uri_encode(account));
    format!(
        "otpauth://totp/{label}?secret={}&issuer={}&algorithm=SHA1&digits=6&period={TOTP_PERIOD}",
        base32(secret),
        uri_encode(issuer)
    )
}

fn uri_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// QR-Code der URI als SVG (ohne Bildbibliothek).
pub fn qr_svg(data: &str) -> ApiResult<String> {
    let code = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M)
        .map_err(|e| anyhow::anyhow!("QR-Code: {e}"))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

// --- Wiederherstellungscodes ----------------------------------------------------------------------

/// Zehn neue Codes im Format `xxxxx-xxxxx` (50 Bit Zufall je Code).
pub fn new_recovery_codes() -> Vec<String> {
    (0..RECOVERY_CODES)
        .map(|_| {
            let mut buf = [0u8; 7];
            rand::fill(&mut buf[..]);
            let s = base32(&buf).to_ascii_lowercase();
            format!("{}-{}", &s[0..5], &s[5..10])
        })
        .collect()
}

/// Codes ohne Groß/Klein, Leerzeichen und Bindestriche vergleichen.
fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn sha256(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

/// Ersetzt alle Wiederherstellungscodes des Nutzers und gibt die neuen (einmalig im Klartext) zurück.
pub async fn replace_recovery_codes(tx: &mut sqlx::PgConnection, user_id: Uuid) -> ApiResult<Vec<String>> {
    let codes = new_recovery_codes();
    sqlx::query("DELETE FROM mfa_recovery_codes WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
    let hashes: Vec<Vec<u8>> =
        codes.iter().map(|c| sha256(&[user_id.as_bytes(), normalize_code(c).as_bytes()])).collect();
    sqlx::query("INSERT INTO mfa_recovery_codes (user_id, code_hash) SELECT $1, unnest($2::bytea[])")
        .bind(user_id)
        .bind(&hashes)
        .execute(&mut *tx)
        .await?;
    Ok(codes)
}

// --- Status -------------------------------------------------------------------------------------------

/// 2FA-Stand eines Kontos (ohne Geheimnisse).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MfaStatus {
    pub totp: bool,
    pub email: bool,
    pub recovery_codes_left: i64,
}

impl MfaStatus {
    pub fn active(&self) -> bool {
        self.totp || self.email
    }
}

pub async fn status(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<MfaStatus> {
    let row: Option<(bool, bool, i64)> = sqlx::query_as(
        "SELECT m.totp_secret IS NOT NULL, m.email_enabled,
                (SELECT count(*) FROM mfa_recovery_codes r WHERE r.user_id = m.user_id AND r.used_at IS NULL)
         FROM user_mfa m WHERE m.user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map_or_else(MfaStatus::default, |(totp, email, left)| MfaStatus {
        totp,
        email,
        recovery_codes_left: left,
    }))
}

/// Methoden für den zweiten Anmeldeschritt. E-Mail nur, wenn Versand möglich ist und eine bestätigte
/// Adresse existiert; Wiederherstellungscodes gehen immer.
pub fn methods(status: &MfaStatus, email_possible: bool) -> Vec<&'static str> {
    let mut m = Vec::with_capacity(3);
    if status.totp {
        m.push("totp");
    }
    if status.email && email_possible {
        m.push("email");
    }
    m.push("recovery");
    m
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Totp,
    Email,
    Recovery,
}

// --- Prüfen -------------------------------------------------------------------------------------------

/// Prüft einen Code und verbraucht ihn (TOTP-Zeitschritt, E-Mail-Code bzw. Wiederherstellungscode).
pub async fn verify(state: &AppState, user_id: Uuid, method: Method, code: &str) -> ApiResult<bool> {
    let code = code.trim();
    match method {
        Method::Totp => {
            let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            let Ok(value) = digits.parse::<u32>() else { return Ok(false) };
            if digits.len() != 6 {
                return Ok(false);
            }
            let row: Option<(Option<Vec<u8>>, Option<i64>)> =
                sqlx::query_as("SELECT totp_secret, totp_last_step FROM user_mfa WHERE user_id = $1")
                    .bind(user_id)
                    .fetch_optional(&state.db)
                    .await?;
            let Some((Some(secret), last)) = row else { return Ok(false) };
            let now = current_step();
            let Some(step) = (now - TOTP_SKEW..=now + TOTP_SKEW)
                .find(|s| last.is_none_or(|l| *s > l) && totp_at(&secret, *s) == value)
            else {
                return Ok(false);
            };
            // Replay-Schutz: Ein Zeitschritt (und alle davor) gilt nur einmal – auch bei parallelen Anfragen.
            let updated = sqlx::query(
                "UPDATE user_mfa SET totp_last_step = $2
                 WHERE user_id = $1 AND (totp_last_step IS NULL OR totp_last_step < $2)",
            )
            .bind(user_id)
            .bind(step)
            .execute(&state.db)
            .await?
            .rows_affected();
            Ok(updated == 1)
        }
        Method::Email => {
            let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
            let hash: Option<Vec<u8>> = sqlx::query_scalar(
                "UPDATE mfa_email_codes SET attempts = attempts + 1
                 WHERE user_id = $1 AND expires_at > now() AND attempts < $2
                 RETURNING code_hash",
            )
            .bind(user_id)
            .bind(EMAIL_CODE_MAX_ATTEMPTS)
            .fetch_optional(&state.db)
            .await?;
            let ok = hash.is_some_and(|h| h == sha256(&[user_id.as_bytes(), digits.as_bytes()]));
            if ok {
                sqlx::query("DELETE FROM mfa_email_codes WHERE user_id = $1")
                    .bind(user_id)
                    .execute(&state.db)
                    .await?;
            }
            Ok(ok)
        }
        Method::Recovery => {
            let hash = sha256(&[user_id.as_bytes(), normalize_code(code).as_bytes()]);
            let used = sqlx::query(
                "UPDATE mfa_recovery_codes SET used_at = now()
                 WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
            )
            .bind(user_id)
            .bind(hash)
            .execute(&state.db)
            .await?
            .rows_affected();
            Ok(used == 1)
        }
    }
}

/// Erzeugt einen 6-stelligen Code und schickt ihn an die bestätigte Adresse des Kontos.
/// `400 email_unavailable`, wenn es keine bestätigte Adresse gibt oder kein Mailversand eingerichtet ist.
pub async fn send_email_code(state: &AppState, user_id: Uuid) -> ApiResult<()> {
    auth::ensure_active(&state.db, user_id).await?;
    if !state.mailer.enabled() {
        return Err(ApiError::BadRequest("email_unavailable"));
    }
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT username, email FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    let Some((username, Some(email))) = row else { return Err(ApiError::BadRequest("email_unavailable")) };
    state.rate_limit.mail_to_account(user_id)?;

    let code = format!("{:06}", rand::random_range(0..1_000_000u32));
    sqlx::query(
        "INSERT INTO mfa_email_codes (user_id, code_hash, attempts, expires_at) VALUES ($1, $2, 0, $3)
         ON CONFLICT (user_id) DO UPDATE SET code_hash = $2, attempts = 0, expires_at = $3",
    )
    .bind(user_id)
    .bind(sha256(&[user_id.as_bytes(), code.as_bytes()]))
    .bind(OffsetDateTime::now_utc() + EMAIL_CODE_TTL)
    .execute(&state.db)
    .await?;
    state.mailer.send(mail::mfa_code(
        &email,
        mail::Instance::from(&*state.config),
        &username,
        &code,
        EMAIL_CODE_TTL,
    ));
    Ok(())
}

// --- Zweiter Anmeldeschritt ---------------------------------------------------------------------------

/// Legt nach korrektem erstem Faktor eine Anmelde-Challenge an und gibt das `mfa_token` zurück.
pub async fn create_challenge(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    session_label: Option<String>,
) -> ApiResult<String> {
    let token = auth::random_token(32);
    let label =
        session_label.map(|l| l.trim().chars().take(64).collect::<String>()).filter(|l| !l.is_empty());
    sqlx::query(
        "INSERT INTO mfa_challenges (token_hash, user_id, session_label, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(sha256(&[token.as_bytes()]))
    .bind(user_id)
    .bind(label)
    .bind(OffsetDateTime::now_utc() + CHALLENGE_TTL)
    .execute(db)
    .await?;
    Ok(token)
}

/// Gültige Challenge zum Token: `(user_id, session_label)`. Zählt einen Versuch; nach zu vielen
/// Fehlversuchen oder Ablauf → `401 invalid_mfa_token`.
pub async fn use_challenge(db: impl PgExecutor<'_>, token: &str) -> ApiResult<(Uuid, Option<String>)> {
    sqlx::query_as(
        "UPDATE mfa_challenges SET attempts = attempts + 1
         WHERE token_hash = $1 AND expires_at > now() AND attempts < $2
         RETURNING user_id, session_label",
    )
    .bind(sha256(&[token.as_bytes()]))
    .bind(CHALLENGE_MAX_ATTEMPTS)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::Unauthorized("invalid_mfa_token"))
}

/// Challenge-Inhaber ohne Versuch zu zählen (z. B. zum Anfordern eines E-Mail-Codes).
pub async fn challenge_user(db: impl PgExecutor<'_>, token: &str) -> ApiResult<Uuid> {
    sqlx::query_scalar(
        "SELECT user_id FROM mfa_challenges WHERE token_hash = $1 AND expires_at > now() AND attempts < $2",
    )
    .bind(sha256(&[token.as_bytes()]))
    .bind(CHALLENGE_MAX_ATTEMPTS)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::Unauthorized("invalid_mfa_token"))
}

pub async fn delete_challenge(db: impl PgExecutor<'_>, token: &str) -> ApiResult<()> {
    sqlx::query("DELETE FROM mfa_challenges WHERE token_hash = $1")
        .bind(sha256(&[token.as_bytes()]))
        .execute(db)
        .await?;
    Ok(())
}

// --- Re-Authentifizierung -----------------------------------------------------------------------------

/// Bestätigung vor sensiblen Aktionen (Passwort, E-Mail, 2FA, Identitäten, Konto löschen).
/// Wird in die jeweiligen Request-Bodies eingebettet (`#[serde(flatten)]`).
#[derive(Debug, Default, Deserialize)]
pub struct Reauth {
    /// Aktuelles Passwort (Pflicht, sofern das Konto eines hat).
    #[serde(default)]
    pub password: Option<String>,
    /// 2FA-Code (Pflicht, sofern 2FA aktiv ist).
    #[serde(default)]
    pub mfa_code: Option<String>,
    /// Art des Codes; Standard: `totp`, falls eingerichtet, sonst `email`.
    #[serde(default)]
    pub mfa_method: Option<Method>,
}

/// Prüft Passwort und ggf. 2FA-Code des angemeldeten Nutzers. Fehlversuche zählen wie Anmeldeversuche
/// (gleiche Limits), damit ein gestohlenes Session-Token nicht zum Durchprobieren taugt.
///
/// Konten ohne Passwort (nur OIDC) brauchen stattdessen eine frische Sitzung (`403 reauth_required`).
pub async fn reauthenticate(
    state: &AppState,
    user: AuthUser,
    ip: Option<std::net::IpAddr>,
    reauth: Reauth,
) -> ApiResult<()> {
    let (username, hash): (String, Option<String>) =
        sqlx::query_as("SELECT username, password_hash FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::Unauthorized("unauthorized"))?;
    state.rate_limit.check_login(&username, ip)?;

    match hash {
        Some(hash) => {
            let password = reauth.password.unwrap_or_default();
            if !auth::verify_password(password, Some(hash)).await? {
                state.rate_limit.login_failed(&username, ip);
                return Err(ApiError::Forbidden("wrong_password"));
            }
        }
        None => {
            let recent: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1 AND created_at > now() - $2::interval)",
            )
            .bind(user.session_id)
            .bind(RECENT_LOGIN)
            .fetch_one(&state.db)
            .await?;
            if !recent {
                return Err(ApiError::Forbidden("reauth_required"));
            }
        }
    }

    let mfa = status(&state.db, user.user_id).await?;
    if mfa.active() {
        let Some(code) = reauth.mfa_code.filter(|c| !c.trim().is_empty()) else {
            return Err(ApiError::Forbidden("mfa_code_required"));
        };
        let method = reauth.mfa_method.unwrap_or(if mfa.totp { Method::Totp } else { Method::Email });
        // Nur eingerichtete Methoden zählen – sonst ließe sich TOTP z. B. per E-Mail-Code umgehen.
        let allowed = match method {
            Method::Totp => mfa.totp,
            Method::Email => mfa.email,
            Method::Recovery => true,
        };
        if !allowed || !verify(state, user.user_id, method, &code).await? {
            state.rate_limit.login_failed(&username, ip);
            return Err(ApiError::Forbidden("invalid_code"));
        }
    }
    state.rate_limit.login_succeeded(&username, ip);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Testvektoren aus RFC 6238, Anhang B (SHA1, 8 Stellen → hier die letzten 6).
    #[test]
    fn totp_rfc6238_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59i64, 94287082u32),
            (1111111109, 7081804),
            (1111111111, 14050471),
            (1234567890, 89005924),
            (2000000000, 69279037),
            (20000000000, 65353130),
        ] {
            assert_eq!(totp_at(secret, time / TOTP_PERIOD), expected % 1_000_000, "t={time}");
        }
    }

    #[test]
    fn base32_rfc4648() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foo"), "MZXW6");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn otpauth() {
        let uri = otpauth_uri("Lykos Test", "alex", b"12345678901234567890");
        assert_eq!(
            uri,
            "otpauth://totp/Lykos%20Test:alex?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Lykos%20Test\
             &algorithm=SHA1&digits=6&period=30"
        );
        let svg = qr_svg(&uri).unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(!svg.contains("<script"));
    }

    #[test]
    fn recovery_codes_format() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODES);
        for c in &codes {
            assert_eq!(c.len(), 11);
            assert_eq!(&c[5..6], "-");
        }
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), RECOVERY_CODES);
        assert_eq!(normalize_code(" ABCDE-fghij "), "abcdefghij");
    }

    #[test]
    fn methods_list() {
        let s = MfaStatus { totp: true, email: true, recovery_codes_left: 3 };
        assert_eq!(methods(&s, true), ["totp", "email", "recovery"]);
        assert_eq!(methods(&s, false), ["totp", "recovery"]);
        assert!(!MfaStatus::default().active());
    }
}
