//! Schlanker Client für externe Anmeldung: OpenID Connect und reines OAuth2 (Authorization Code mit
//! `state` und PKCE S256).
//!
//! Bewusst selbst gebaut statt `openidconnect`/`oauth2`: Wir brauchen nur Discovery, den Token-Endpunkt,
//! die ID-Token-Prüfung und – für reines OAuth2 (Discord, GitHub) – den Userinfo-Endpunkt. Die
//! Signaturprüfung übernimmt `jsonwebtoken` (RustCrypto), HTTP läuft über das ohnehin vorhandene
//! `reqwest` 0.13 mit rustls – keine zweite HTTP-/TLS-Stack-Version im Build.
//!
//! OIDC: Geprüft werden Signatur (nur asymmetrische Verfahren, Schlüssel aus dem JWKS des Anbieters), `iss`
//! (exakt der konfigurierte Issuer), `aud` (enthält unsere Client-ID; bei mehreren Audiences muss `azp`
//! unsere Client-ID sein), `exp` (60 s Toleranz), `nonce` (aus dem Start der Anmeldung) und `sub`.
//!
//! OAuth2: Es gibt kein ID-Token; die Identität ist die stabile ID aus dem Userinfo-Endpunkt, abgerufen
//! mit dem Access-Token direkt beim Anbieter (über TLS). Gegen CSRF schützen `state` (an den Browser
//! gebunden) und PKCE (Discord und GitHub unterstützen es; Anbieter ohne PKCE ignorieren die Parameter).
//!
//! Vom Anbieter übernommen wird ausschließlich die ID – und die E-Mail-Adresse nur, wenn sie als
//! bestätigt gemeldet ist und der Nutzer zustimmt.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::{AuthProtocol, OAuth2Endpoints, OidcProviderConfig};

/// So lange gelten Discovery-Dokument und Schlüssel, bevor sie neu geladen werden.
const METADATA_TTL: Duration = Duration::from_secs(60 * 60);
/// Erlaubte Signaturverfahren (keine HMAC-Verfahren, kein `none`).
const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// Discovery-Daten und Schlüssel eines OIDC-Anbieters.
#[derive(Debug, Clone)]
pub struct ProviderMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub token_endpoint_auth_methods: Vec<String>,
    pub jwks: JwkSet,
    fetched: Instant,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Vec<String>,
}

/// Die für Lykos relevanten Angaben einer geprüften Anmeldung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityClaims {
    pub subject: String,
    /// Nur gesetzt, wenn der Anbieter die Adresse als bestätigt meldet.
    pub verified_email: Option<String>,
}

#[derive(Deserialize)]
struct RawClaims {
    sub: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    aud: serde_json::Value,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// Manche Anbieter senden `"true"` als Zeichenkette.
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
}

/// Client mit Cache für Discovery und Schlüssel der OIDC-Anbieter (pro Prozess).
#[derive(Default)]
pub struct Oidc {
    cache: Mutex<HashMap<String, Arc<ProviderMetadata>>>,
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::install_crypto_provider();
        // GitHub verlangt einen User-Agent.
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("lykos-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP-Client")
    })
}

fn is_true(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

impl Oidc {
    /// Discovery-Dokument und JWKS eines OIDC-Anbieters (aus dem Cache, falls frisch genug).
    pub async fn metadata(&self, id: &str, issuer: &str, refresh: bool) -> Result<Arc<ProviderMetadata>> {
        if !refresh
            && let Some(meta) = self.lock().get(id).cloned()
            && meta.fetched.elapsed() < METADATA_TTL
        {
            return Ok(meta);
        }
        let url = format!("{issuer}/.well-known/openid-configuration");
        let discovery: Discovery = http()
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("OIDC-Discovery {url}"))?
            .json()
            .await
            .context("OIDC-Discovery: ungültiges JSON")?;
        // Der Issuer im Dokument muss exakt der konfigurierte sein (OpenID Connect Discovery 1.0, 4.3).
        if discovery.issuer.trim_end_matches('/') != issuer {
            bail!("OIDC-Discovery: Issuer {:?} passt nicht zur Konfiguration", discovery.issuer);
        }
        let jwks: JwkSet = http()
            .get(&discovery.jwks_uri)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .context("OIDC-JWKS laden")?
            .json()
            .await
            .context("OIDC-JWKS: ungültiges JSON")?;
        let meta = Arc::new(ProviderMetadata {
            issuer: discovery.issuer,
            authorization_endpoint: discovery.authorization_endpoint,
            token_endpoint: discovery.token_endpoint,
            jwks_uri: discovery.jwks_uri,
            token_endpoint_auth_methods: discovery.token_endpoint_auth_methods_supported,
            jwks,
            fetched: Instant::now(),
        });
        self.lock().insert(id.to_owned(), meta.clone());
        Ok(meta)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<ProviderMetadata>>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Autorisierungs-URL für den Browser (lädt bei OIDC ggf. das Discovery-Dokument).
    pub async fn authorization_url(
        &self,
        provider: &OidcProviderConfig,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<String> {
        let challenge = pkce_challenge(pkce_verifier);
        let scope = provider.scopes.join(" ");
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", provider.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ];
        if !scope.is_empty() {
            params.push(("scope", &scope));
        }
        let endpoint = match &provider.protocol {
            AuthProtocol::Oidc { issuer } => {
                params.push(("nonce", nonce));
                self.metadata(&provider.id, issuer, false).await?.authorization_endpoint.clone()
            }
            AuthProtocol::OAuth2(endpoints) => endpoints.authorize_url.clone(),
        };
        let sep = if endpoint.contains('?') { '&' } else { '?' };
        Ok(format!("{endpoint}{sep}{}", form_encode(&params)))
    }

    /// Tauscht den Code gegen Tokens und gibt die geprüfte Identität zurück.
    pub async fn complete(
        &self,
        provider: &OidcProviderConfig,
        code: &str,
        pkce_verifier: &str,
        redirect_uri: &str,
        nonce: &str,
    ) -> Result<IdentityClaims> {
        match &provider.protocol {
            AuthProtocol::Oidc { issuer } => {
                let meta = self.metadata(&provider.id, issuer, false).await?;
                // client_secret_basic ist der Standard (RFC 6749 2.3.1); client_secret_post nur, wenn der
                // Anbieter Basic ausdrücklich nicht anbietet.
                let basic = meta.token_endpoint_auth_methods.is_empty()
                    || meta.token_endpoint_auth_methods.iter().any(|m| m == "client_secret_basic");
                let tokens =
                    exchange_code(provider, &meta.token_endpoint, basic, code, pkce_verifier, redirect_uri)
                        .await?;
                let id_token = tokens.id_token.context("Token-Antwort ohne id_token")?;
                match validate_id_token(&id_token, &meta.jwks, issuer, &provider.client_id, nonce) {
                    Err(e) if e.downcast_ref::<UnknownKey>().is_some() => {
                        // Schlüsselwechsel beim Anbieter: JWKS einmal neu laden.
                        let meta = self.metadata(&provider.id, issuer, true).await?;
                        validate_id_token(&id_token, &meta.jwks, issuer, &provider.client_id, nonce)
                    }
                    other => other,
                }
            }
            AuthProtocol::OAuth2(endpoints) => {
                // Discord und GitHub erwarten die Client-Daten im Body (client_secret_post).
                let tokens =
                    exchange_code(provider, &endpoints.token_url, false, code, pkce_verifier, redirect_uri)
                        .await?;
                let access_token = tokens.access_token.context("Token-Antwort ohne access_token")?;
                let info: serde_json::Value = http()
                    .get(&endpoints.userinfo_url)
                    .bearer_auth(access_token)
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .context("Userinfo-Endpunkt")?
                    .json()
                    .await
                    .context("Userinfo: ungültiges JSON")?;
                identity_from_userinfo(endpoints, &info)
            }
        }
    }
}

/// Identität aus der Userinfo-Antwort eines OAuth2-Anbieters.
pub fn identity_from_userinfo(
    endpoints: &OAuth2Endpoints,
    info: &serde_json::Value,
) -> Result<IdentityClaims> {
    let subject = match info.get(&endpoints.subject_field) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => bail!("Userinfo ohne {:?}", endpoints.subject_field),
    };
    if subject.is_empty() || subject.len() > 255 {
        bail!("Userinfo: ungültige ID");
    }
    let verified_email = match (&endpoints.email_field, &endpoints.email_verified_field) {
        (Some(email), Some(verified)) if is_true(info.get(verified)) => info
            .get(email)
            .and_then(|e| e.as_str())
            .map(|e| e.trim().to_owned())
            .filter(|e| crate::mail::is_valid_email(e)),
        _ => None,
    };
    Ok(IdentityClaims { subject, verified_email })
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    /// GitHub meldet Fehler mit Status 200 und `error`.
    #[serde(default)]
    error: Option<String>,
}

async fn exchange_code(
    provider: &OidcProviderConfig,
    token_endpoint: &str,
    basic_auth: bool,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", pkce_verifier),
    ];
    let mut req = http().post(token_endpoint).header("Accept", "application/json");
    match &provider.client_secret {
        Some(secret) if basic_auth => {
            let credentials = format!("{}:{}", url_encode(&provider.client_id), url_encode(secret));
            req = req.header("Authorization", format!("Basic {}", STANDARD.encode(credentials)));
        }
        Some(secret) => {
            params.push(("client_id", &provider.client_id));
            params.push(("client_secret", secret));
        }
        None => params.push(("client_id", &provider.client_id)),
    }
    let res = req
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_encode(&params))
        .send()
        .await
        .context("Token-Endpunkt nicht erreichbar")?;
    if !res.status().is_success() {
        bail!("Token-Endpunkt antwortet {}", res.status());
    }
    let body: TokenResponse = res.json().await.context("Token-Antwort: ungültiges JSON")?;
    if let Some(error) = &body.error {
        bail!("Token-Endpunkt meldet Fehler {error:?}");
    }
    Ok(body)
}

/// Kein passender Schlüssel im JWKS (→ einmal neu laden).
#[derive(Debug)]
struct UnknownKey;

impl std::fmt::Display for UnknownKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ID-Token: Schlüssel unbekannt")
    }
}

impl std::error::Error for UnknownKey {}

/// Prüft ein ID-Token gegen JWKS, Issuer, Client-ID und Nonce.
pub fn validate_id_token(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    client_id: &str,
    nonce: &str,
) -> Result<IdentityClaims> {
    let header = jsonwebtoken::decode_header(token).context("ID-Token: ungültiger Header")?;
    if !ALLOWED_ALGORITHMS.contains(&header.alg) {
        bail!("ID-Token: Verfahren {:?} nicht erlaubt", header.alg);
    }
    let jwk = match &header.kid {
        Some(kid) => jwks.find(kid),
        // Ohne `kid` nur eindeutig, wenn der Anbieter genau einen Schlüssel veröffentlicht.
        None if jwks.keys.len() == 1 => jwks.keys.first(),
        None => None,
    }
    .ok_or(UnknownKey)?;
    let key = DecodingKey::from_jwk(jwk).context("ID-Token: Schlüssel unbrauchbar")?;

    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer, &format!("{issuer}/")]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = 60;
    let claims =
        jsonwebtoken::decode::<RawClaims>(token, &key, &validation).context("ID-Token ungültig")?.claims;

    if claims.nonce.as_deref() != Some(nonce) {
        bail!("ID-Token: nonce passt nicht");
    }
    let multiple_audiences = claims.aud.as_array().is_some_and(|a| a.len() > 1);
    if (multiple_audiences || claims.azp.is_some()) && claims.azp.as_deref() != Some(client_id) {
        bail!("ID-Token: azp passt nicht");
    }
    if claims.sub.is_empty() || claims.sub.len() > 255 {
        bail!("ID-Token: ungültiges sub");
    }
    let verified = is_true(claims.email_verified.as_ref());
    let verified_email =
        claims.email.filter(|e| verified && crate::mail::is_valid_email(e)).map(|e| e.trim().to_owned());
    Ok(IdentityClaims { subject: claims.sub, verified_email })
}

/// PKCE-Code-Challenge (S256) zu einem Verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn form_encode(params: &[(&str, &str)]) -> String {
    params.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&")
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{EncodingKey, Header, jwk::Jwk};
    use serde_json::json;

    use super::*;

    /// Wegwerf-Schlüssel nur für Tests: Ed25519, PKCS#8-DER als Base64.
    const TEST_KEY: &str = "MC4CAQAwBQYDK2VwBCIEINnINSmJysz0U5BBYOOvuBkqbjnrfQCPtXij/dWy1X4u";

    fn test_key() -> EncodingKey {
        EncodingKey::from_ed_der(&STANDARD.decode(TEST_KEY).unwrap())
    }
    const TEST_X: &str = "XlhEyP776J7FlZQ1PGee51C3oP0DhBNN69x0E8et07Y";
    const ISS: &str = "https://sso.example.org";

    fn jwks() -> JwkSet {
        serde_json::from_value(json!({ "keys": [
            { "kty": "OKP", "crv": "Ed25519", "x": TEST_X, "kid": "k1", "alg": "EdDSA", "use": "sig" }
        ]}))
        .unwrap()
    }

    fn token(claims: serde_json::Value, kid: Option<&str>) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = kid.map(String::from);
        jsonwebtoken::encode(&header, &claims, &test_key()).unwrap()
    }

    fn claims() -> serde_json::Value {
        let now = jsonwebtoken::get_current_timestamp();
        json!({ "iss": ISS, "aud": "lykos", "sub": "user-1", "exp": now + 300, "iat": now, "nonce": "n1",
                "email": "a@example.org", "email_verified": true })
    }

    fn with(mut c: serde_json::Value, key: &str, value: serde_json::Value) -> serde_json::Value {
        c[key] = value;
        c
    }

    #[test]
    fn valid_token() {
        let c = validate_id_token(&token(claims(), Some("k1")), &jwks(), ISS, "lykos", "n1").unwrap();
        assert_eq!(c.subject, "user-1");
        assert_eq!(c.verified_email.as_deref(), Some("a@example.org"));
        // Unbestätigte Adresse wird nicht übernommen.
        let t = token(with(claims(), "email_verified", json!(false)), Some("k1"));
        assert_eq!(validate_id_token(&t, &jwks(), ISS, "lykos", "n1").unwrap().verified_email, None);
        // Ohne kid geht es nur bei genau einem Schlüssel.
        assert!(validate_id_token(&token(claims(), None), &jwks(), ISS, "lykos", "n1").is_ok());
    }

    #[test]
    fn rejects_wrong_claims() {
        let check =
            |c: serde_json::Value| validate_id_token(&token(c, Some("k1")), &jwks(), ISS, "lykos", "n1");
        assert!(check(with(claims(), "iss", json!("https://evil.example.org"))).is_err());
        assert!(check(with(claims(), "aud", json!("andere-app"))).is_err());
        assert!(check(with(claims(), "nonce", json!("n2"))).is_err());
        let mut no_nonce = claims();
        no_nonce.as_object_mut().unwrap().remove("nonce");
        assert!(check(no_nonce).is_err());
        let expired = jsonwebtoken::get_current_timestamp() - 600;
        assert!(check(with(claims(), "exp", json!(expired))).is_err());
        // Mehrere Audiences ohne passendes azp.
        assert!(check(with(claims(), "aud", json!(["lykos", "andere"]))).is_err());
        let multi = with(with(claims(), "aud", json!(["lykos", "andere"])), "azp", json!("lykos"));
        assert!(check(multi).is_ok());
    }

    #[test]
    fn rejects_unknown_key_and_bad_signature() {
        let err = validate_id_token(&token(claims(), Some("k2")), &jwks(), ISS, "lykos", "n1").unwrap_err();
        assert!(err.downcast_ref::<UnknownKey>().is_some());
        // Manipulierte Nutzlast → Signatur passt nicht.
        let t = token(claims(), Some("k1"));
        let mut parts: Vec<&str> = t.split('.').collect();
        let forged =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&with(claims(), "sub", json!("admin"))).unwrap());
        parts[1] = &forged;
        assert!(validate_id_token(&parts.join("."), &jwks(), ISS, "lykos", "n1").is_err());
        // HS256 (symmetrisch) wird grundsätzlich abgelehnt.
        let hs =
            jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims(), &EncodingKey::from_secret(b"x"))
                .unwrap();
        assert!(validate_id_token(&hs, &jwks(), ISS, "lykos", "n1").is_err());
    }

    #[test]
    fn jwk_roundtrip_matches_constant() {
        // Sicherstellen, dass der öffentliche Testschlüssel zum privaten passt.
        let jwk = Jwk::from_encoding_key(&test_key(), Algorithm::EdDSA).unwrap();
        let value = serde_json::to_value(&jwk).unwrap();
        assert_eq!(value["x"], TEST_X);
    }

    #[test]
    fn userinfo_identity() {
        let endpoints = OAuth2Endpoints {
            authorize_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            subject_field: "id".into(),
            email_field: Some("email".into()),
            email_verified_field: Some("verified".into()),
        };
        // Discord: ID als Zeichenkette, Adresse nur wenn bestätigt.
        let c = identity_from_userinfo(
            &endpoints,
            &json!({ "id": "80351110224678912", "username": "wolf", "email": "w@example.org", "verified": true }),
        )
        .unwrap();
        assert_eq!(c.subject, "80351110224678912");
        assert_eq!(c.verified_email.as_deref(), Some("w@example.org"));
        let c = identity_from_userinfo(
            &endpoints,
            &json!({ "id": "1", "email": "w@example.org", "verified": false }),
        )
        .unwrap();
        assert_eq!(c.verified_email, None);
        // GitHub: numerische ID.
        assert_eq!(identity_from_userinfo(&endpoints, &json!({ "id": 583231 })).unwrap().subject, "583231");
        assert!(identity_from_userinfo(&endpoints, &json!({ "login": "octocat" })).is_err());
        assert!(identity_from_userinfo(&endpoints, &json!({ "id": "" })).is_err());
    }

    #[test]
    fn pkce_and_form() {
        // RFC 7636, Anhang B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(form_encode(&[("a b", "c&d=é")]), "a%20b=c%26d%3D%C3%A9");
    }
}
