//! Instanz-Konfiguration, ausschließlich aus Umgebungsvariablen.
//!
//! Alles Instanzspezifische (Hostname, Limits, Registrierungsmodus, ...) gehört hierher und
//! nicht in den Code – damit dieselbe Binary für die zentrale Instanz und für
//! Single-Tenant-Deployments taugt.

use std::{
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, Result};

/// Offizieller Quellcode von Lykos – Standard für `LYKOS_SOURCE_URL`.
pub const DEFAULT_SOURCE_URL: &str = "https://github.com/ZeasySoftware/Lykos";

#[derive(Debug, Clone)]
pub struct Config {
    /// Adresse, auf der der HTTP-Server lauscht.
    pub bind: SocketAddr,
    /// Anzeigename der Instanz.
    pub instance_name: String,
    /// Quellcode dieser Instanz (`LYKOS_SOURCE_URL`). Die AGPL verlangt, Nutzern eines Netzwerkdienstes den
    /// Quellcode der laufenden Fassung anzubieten – wer Lykos verändert betreibt, trägt hier seinen Fork ein.
    pub source_url: String,
    /// Postgres-Verbindungs-URL.
    pub database_url: String,
    /// Öffentliche Basis-URL der Instanz, z. B. `https://lykos.zeasy.dev`.
    pub public_url: String,
    /// Verzeichnis mit dem gebauten Web-Client; `None` = API-only.
    pub web_dir: Option<PathBuf>,
    /// Zusätzlich erlaubte CORS-Origins (Desktop-Client, Dev-Server).
    pub cors_origins: Vec<String>,
    /// Läuft Lykos hinter einem vertrauenswürdigen Reverse Proxy? Dann gilt die Client-IP aus
    /// `X-Forwarded-For` (nur fürs Rate-Limiting, wird nicht gespeichert). `LYKOS_TRUST_PROXY`.
    /// Ohne `trusted_proxies` von **jeder** Verbindung – fälschbar, wenn der Port direkt erreichbar ist.
    pub trust_proxy: bool,
    /// Adressen der Reverse Proxys (`LYKOS_TRUSTED_PROXIES`, IPs bzw. CIDRs, kommagetrennt). Gesetzt,
    /// gilt `X-Forwarded-For` nur für Verbindungen von dort – unabhängig von `trust_proxy`.
    pub trusted_proxies: Vec<IpNet>,
    /// Verzeichnis für hochgeladene Dateien (lokaler Speicher, `LYKOS_STORAGE=local`).
    pub storage_dir: PathBuf,
    /// Speicher-Backend für Anhänge (`LYKOS_STORAGE`).
    pub storage: StorageBackend,
    /// Maximale Größe eines Uploads in Bytes.
    pub max_upload_bytes: u64,
    /// Speicher-Kontingent pro Nutzer für Nachrichten-Anhänge in Bytes; `None` = unbegrenzt.
    /// `LYKOS_USER_QUOTA_MB` (Standard 2048, `0` = unbegrenzt).
    pub user_quota_bytes: Option<u64>,
    /// LiveKit-Anbindung für Voice/Screenshare; `None` = Voice deaktiviert.
    pub livekit: Option<LiveKitConfig>,
    /// Gitea-API-URL der Release-Liste des öffentlichen Release-Repos (Desktop-Download-Seite und
    /// Update-Manifest), z. B. `https://api.github.com/repos/ZeasySoftware/Lykos/releases` (GitHub) oder `https://git.example.org/api/v1/repos/<besitzer>/<repo>/releases` (Gitea).
    /// `None` = keine Desktop-Downloads/Updates über diese Instanz. `LYKOS_DESKTOP_RELEASES_URL`.
    pub desktop_releases_url: Option<String>,
    /// Mailversand (Bestätigungslinks, Passwort-Reset, 2FA-Codes). `LYKOS_MAIL`, `LYKOS_SMTP_*`,
    /// `LYKOS_MAIL_FROM`; ohne Konfiguration deaktiviert.
    pub mail: MailConfig,
    /// Vom Betreiber konfigurierte Anmeldeanbieter (OIDC/OAuth2, `LYKOS_OAUTH_PROVIDERS`).
    pub oidc_providers: Vec<OidcProviderConfig>,
    /// Optionale Weiterleitung von Feedback-Meldungen als Gitea-Issue (nur per Klick eines Admins).
    /// `LYKOS_FEEDBACK_GITEA_URL` + `LYKOS_FEEDBACK_GITEA_TOKEN`; `None` = Funktion ausgeblendet.
    pub feedback_gitea: Option<FeedbackGiteaConfig>,
    /// So lange klingelt ein DM-Anruf bei den Angerufenen (Sekunden, fest `DEFAULT_CALL_RING_SECS`).
    pub call_ring_secs: u64,
}

/// Klingeldauer eines DM-Anrufs, wenn niemand annimmt oder ablehnt.
pub const DEFAULT_CALL_RING_SECS: u64 = 40;

/// Wie Mails verschickt werden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailConfig {
    /// Keine Mails – E-Mail-Adresse, Passwort-Reset und 2FA per E-Mail sind ausgeblendet.
    Disabled,
    /// Nur Entwicklung: Mails (inkl. Links und Codes!) landen im Log.
    Log,
    /// Versand über SMTP.
    Smtp(SmtpConfig),
}

/// SMTP-Zugang. Verschlüsselung: `smtps://` (implizites TLS, meist Port 465) oder STARTTLS (587).
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    /// `None` = Standardport der TLS-Art.
    pub port: Option<u16>,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Absenderadresse (ohne Namen; angezeigt wird „Lykos“).
    pub from: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// Implizites TLS (`smtps://`).
    Tls,
    /// STARTTLS, Pflicht (`smtp://`).
    StartTls,
    /// Unverschlüsselt – nur für lokale Relays (`LYKOS_SMTP_TLS=none`).
    None,
}

impl std::fmt::Debug for SmtpConfig {
    /// Ohne Passwort.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("from", &self.from)
            .finish()
    }
}

impl MailConfig {
    /// Liest `LYKOS_MAIL` (`smtp` | `log` | `off`), `LYKOS_SMTP_URL` bzw. `LYKOS_SMTP_HOST/_PORT/_USER/_PASSWORD/_TLS`
    /// und `LYKOS_MAIL_FROM` über `get`. Ohne Angaben: `Disabled`.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let url = get("LYKOS_SMTP_URL");
        let host = get("LYKOS_SMTP_HOST");
        let kind = get("LYKOS_MAIL")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_else(|| if url.is_some() || host.is_some() { "smtp".into() } else { "off".into() });
        match kind.as_str() {
            "off" | "none" | "disabled" => Ok(Self::Disabled),
            "log" => Ok(Self::Log),
            "smtp" => {
                let from = get("LYKOS_MAIL_FROM").context("LYKOS_MAIL_FROM fehlt (Mailversand per SMTP)")?;
                if !crate::mail::is_valid_email(&from) {
                    anyhow::bail!("LYKOS_MAIL_FROM ist keine gültige E-Mail-Adresse");
                }
                let mut smtp = match (url, host) {
                    (Some(url), _) => parse_smtp_url(&url)?,
                    (None, Some(host)) => {
                        let tls = match get("LYKOS_SMTP_TLS").map(|v| v.to_ascii_lowercase()).as_deref() {
                            None | Some("implicit" | "tls" | "smtps") => SmtpTls::Tls,
                            Some("starttls") => SmtpTls::StartTls,
                            Some("none") => SmtpTls::None,
                            Some(_) => anyhow::bail!("LYKOS_SMTP_TLS muss implicit, starttls oder none sein"),
                        };
                        let port = get("LYKOS_SMTP_PORT")
                            .map(|p| p.parse().context("LYKOS_SMTP_PORT ist keine gültige Portnummer"))
                            .transpose()?;
                        SmtpConfig {
                            host,
                            port,
                            tls,
                            username: get("LYKOS_SMTP_USER"),
                            password: get("LYKOS_SMTP_PASSWORD"),
                            from: String::new(),
                        }
                    }
                    (None, None) => {
                        anyhow::bail!("LYKOS_MAIL=smtp braucht LYKOS_SMTP_URL oder LYKOS_SMTP_HOST")
                    }
                };
                smtp.from = from;
                Ok(Self::Smtp(smtp))
            }
            other => anyhow::bail!("LYKOS_MAIL muss smtp, log oder off sein, nicht {other:?}"),
        }
    }
}

/// `smtps://user:pass@host:465` (TLS) oder `smtp://user:pass@host:587` (STARTTLS).
/// Benutzer und Passwort dürfen URL-kodiert sein (`%40` für `@`).
fn parse_smtp_url(url: &str) -> Result<SmtpConfig> {
    let (tls, rest) = if let Some(rest) = url.strip_prefix("smtps://") {
        (SmtpTls::Tls, rest)
    } else if let Some(rest) = url.strip_prefix("smtp://") {
        (SmtpTls::StartTls, rest)
    } else {
        anyhow::bail!("LYKOS_SMTP_URL muss mit smtps:// oder smtp:// beginnen");
    };
    let rest = rest.split(['/', '?']).next().unwrap_or_default();
    let (userinfo, hostport) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, rest),
    };
    let (username, password) = match userinfo {
        Some(u) => {
            let (user, pass) = u.split_once(':').map_or((u, None), |(a, b)| (a, Some(b)));
            (Some(percent_decode(user)?), pass.map(percent_decode).transpose()?)
        }
        None => (None, None),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !p.contains(']') => {
            (h.to_owned(), Some(p.parse::<u16>().context("LYKOS_SMTP_URL: ungültiger Port")?))
        }
        _ => (hostport.to_owned(), None),
    };
    if host.is_empty() {
        anyhow::bail!("LYKOS_SMTP_URL: Host fehlt");
    }
    Ok(SmtpConfig { host, port, tls, username, password, from: String::new() })
}

fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3).context("LYKOS_SMTP_URL: ungültige %-Kodierung")?;
            out.push(u8::from_str_radix(hex, 16).context("LYKOS_SMTP_URL: ungültige %-Kodierung")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).context("LYKOS_SMTP_URL: kein UTF-8")
}

/// Ein externer Anmeldeanbieter: OpenID Connect (Discovery + ID-Token) oder reines OAuth2 (Identität über
/// den Userinfo-Endpunkt). Immer Authorization Code mit `state` und PKCE (S256).
#[derive(Clone, PartialEq, Eq)]
pub struct OidcProviderConfig {
    /// Kennung in URLs und in `user_identities.provider`, z. B. `discord` (klein, `[a-z0-9_-]`).
    pub id: String,
    /// Name auf dem Button („Mit … anmelden“).
    pub label: String,
    pub client_id: String,
    /// `None` = öffentlicher Client (nur PKCE).
    pub client_secret: Option<String>,
    /// Angefragte Scopes.
    pub scopes: Vec<String>,
    pub protocol: AuthProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthProtocol {
    /// Issuer-URL; Discovery unter `{issuer}/.well-known/openid-configuration`.
    Oidc { issuer: String },
    /// Reines OAuth2 ohne ID-Token.
    OAuth2(OAuth2Endpoints),
}

/// Endpunkte und Feldnamen eines reinen OAuth2-Anbieters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuth2Endpoints {
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    /// Feld der Userinfo-Antwort mit der stabilen Nutzer-ID (Zahl oder Zeichenkette), z. B. `id`.
    pub subject_field: String,
    /// Feld mit der E-Mail-Adresse – übernommen nur, wenn `email_verified_field` `true` ist.
    pub email_field: Option<String>,
    pub email_verified_field: Option<String>,
}

impl std::fmt::Debug for OidcProviderConfig {
    /// Ohne Client-Secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProviderConfig")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .field("scopes", &self.scopes)
            .field("protocol", &self.protocol)
            .finish()
    }
}

/// Fertige Voreinstellungen: Der Betreiber trägt nur Client-ID und -Secret ein.
fn preset(id: &str) -> Option<(&'static str, AuthProtocol, &'static [&'static str])> {
    let oauth2 = |authorize: &str, token: &str, userinfo: &str, email: Option<(&str, &str)>| {
        AuthProtocol::OAuth2(OAuth2Endpoints {
            authorize_url: authorize.into(),
            token_url: token.into(),
            userinfo_url: userinfo.into(),
            subject_field: "id".into(),
            email_field: email.map(|(e, _)| e.into()),
            email_verified_field: email.map(|(_, v)| v.into()),
        })
    };
    Some(match id {
        // Scope `identify` liefert nur die ID; mit `LYKOS_OAUTH_DISCORD_SCOPES="identify email"` zusätzlich die
        // (von Discord bestätigte) Adresse, die nur nach Zustimmung übernommen wird.
        "discord" => (
            "Discord",
            oauth2(
                "https://discord.com/oauth2/authorize",
                "https://discord.com/api/oauth2/token",
                "https://discord.com/api/users/@me",
                Some(("email", "verified")),
            ),
            &["identify"],
        ),
        // Ohne Scope nur öffentliche Profildaten; die numerische `id` ist stabil (Login-Namen nicht).
        "github" => (
            "GitHub",
            oauth2(
                "https://github.com/login/oauth/authorize",
                "https://github.com/login/oauth/access_token",
                "https://api.github.com/user",
                None,
            ),
            &[],
        ),
        "google" => (
            "Google",
            AuthProtocol::Oidc { issuer: "https://accounts.google.com".into() },
            &["openid", "email"],
        ),
        _ => return None,
    })
}

/// Nur https – außer für lokale Test-Anbieter.
fn require_https(name: &str, url: String) -> Result<String> {
    let url = url.trim_end_matches('/').to_owned();
    if url.starts_with("https://") || is_local_http(&url) {
        Ok(url)
    } else {
        anyhow::bail!("{name} muss eine https://-URL sein")
    }
}

impl OidcProviderConfig {
    /// Liest `LYKOS_OAUTH_PROVIDERS=discord,github,google,firma` und je Anbieter `LYKOS_OAUTH_<ID>_CLIENT_ID`,
    /// `_CLIENT_SECRET`, optional `_LABEL` und `_SCOPES` (Leerzeichen-getrennt).
    ///
    /// `discord`, `github` und `google` sind Voreinstellungen. Andere Kennungen brauchen entweder
    /// `_ISSUER` (generisches OIDC) oder `_AUTHORIZE_URL`, `_TOKEN_URL`, `_USERINFO_URL` (reines OAuth2,
    /// optional `_SUBJECT_FIELD` = `id`, `_EMAIL_FIELD`, `_EMAIL_VERIFIED_FIELD`).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Vec<Self>> {
        let get = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let Some(list) = get("LYKOS_OAUTH_PROVIDERS") else { return Ok(vec![]) };
        let mut providers: Vec<Self> = Vec::new();
        for id in list.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
            if id.len() > 32 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                anyhow::bail!("LYKOS_OAUTH_PROVIDERS: ungültige Kennung {id:?} (erlaubt: a-z, 0-9, _ und -)");
            }
            if providers.iter().any(|p| p.id == id) {
                anyhow::bail!("LYKOS_OAUTH_PROVIDERS: {id:?} doppelt");
            }
            let prefix = format!("LYKOS_OAUTH_{}_", id.to_ascii_uppercase().replace('-', "_"));
            let var = |suffix: &str| get(&format!("{prefix}{suffix}"));
            let require = |suffix: &str| var(suffix).with_context(|| format!("{prefix}{suffix} fehlt"));
            let url =
                |suffix: &str| require(suffix).and_then(|u| require_https(&format!("{prefix}{suffix}"), u));

            let (default_label, protocol, default_scopes): (String, AuthProtocol, Vec<String>) =
                if let Some((label, protocol, scopes)) = preset(&id) {
                    (label.into(), protocol, scopes.iter().map(|s| (*s).into()).collect())
                } else if var("ISSUER").is_some() {
                    let issuer = url("ISSUER")?;
                    (id.clone(), AuthProtocol::Oidc { issuer }, vec!["openid".into(), "email".into()])
                } else if var("AUTHORIZE_URL").is_some() {
                    let endpoints = OAuth2Endpoints {
                        authorize_url: url("AUTHORIZE_URL")?,
                        token_url: url("TOKEN_URL")?,
                        userinfo_url: url("USERINFO_URL")?,
                        subject_field: var("SUBJECT_FIELD").unwrap_or_else(|| "id".into()),
                        email_field: var("EMAIL_FIELD"),
                        email_verified_field: var("EMAIL_VERIFIED_FIELD"),
                    };
                    (id.clone(), AuthProtocol::OAuth2(endpoints), vec![])
                } else {
                    anyhow::bail!("{prefix}ISSUER (OIDC) oder {prefix}AUTHORIZE_URL (OAuth2) fehlt");
                };
            let scopes =
                var("SCOPES").map_or(default_scopes, |s| s.split_whitespace().map(String::from).collect());
            if matches!(protocol, AuthProtocol::Oidc { .. }) && !scopes.iter().any(|s| s == "openid") {
                anyhow::bail!("{prefix}SCOPES muss openid enthalten");
            }
            providers.push(Self {
                label: var("LABEL").unwrap_or(default_label),
                client_id: require("CLIENT_ID")?,
                client_secret: var("CLIENT_SECRET"),
                scopes,
                protocol,
                id,
            });
        }
        Ok(providers)
    }
}

/// `http://` ist nur für lokale Test-Anbieter erlaubt.
fn is_local_http(url: &str) -> bool {
    ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|p| {
        url.strip_prefix(p)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with('/'))
    })
}

/// Zugang zum Gitea-Repo, in dem Admins Feedback-Meldungen als Issue anlegen können.
#[derive(Clone, PartialEq, Eq)]
pub struct FeedbackGiteaConfig {
    /// API-Basis des Repos, z. B. `https://git.example.org/api/v1/repos/<besitzer>/<repo>`.
    pub api_url: String,
    /// Zugriffstoken mit Schreibrecht auf Issues.
    pub token: String,
}

impl std::fmt::Debug for FeedbackGiteaConfig {
    /// Ohne Token – die Konfiguration landet sonst womöglich im Log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedbackGiteaConfig").field("api_url", &self.api_url).field("token", &"***").finish()
    }
}

impl FeedbackGiteaConfig {
    /// Liest `LYKOS_FEEDBACK_GITEA_URL` und `LYKOS_FEEDBACK_GITEA_TOKEN` über `get`. Nur beide zusammen
    /// aktivieren die Funktion; eine ohne die andere ist ein Konfigurationsfehler.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let get = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        match (get("LYKOS_FEEDBACK_GITEA_URL"), get("LYKOS_FEEDBACK_GITEA_TOKEN")) {
            (None, None) => Ok(None),
            (Some(url), Some(token)) => {
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    anyhow::bail!("LYKOS_FEEDBACK_GITEA_URL muss eine http(s)-URL sein");
                }
                Ok(Some(Self { api_url: url.trim_end_matches('/').to_owned(), token }))
            }
            _ => anyhow::bail!("LYKOS_FEEDBACK_GITEA_URL und LYKOS_FEEDBACK_GITEA_TOKEN nur zusammen setzen"),
        }
    }
}

/// Zugangsdaten und Adressen des LiveKit-SFU.
#[derive(Clone)]
pub struct LiveKitConfig {
    /// Öffentliche Signalisierungs-URL für Clients, z. B. `wss://rtc.chat.example.org`.
    pub url: String,
    /// Interne HTTP-URL für die Server-API (RoomService), z. B. `http://127.0.0.1:7880`.
    pub api_url: String,
    pub api_key: String,
    pub api_secret: String,
}

impl std::fmt::Debug for LiveKitConfig {
    /// Ohne API-Secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveKitConfig")
            .field("url", &self.url)
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key)
            .field("api_secret", &"***")
            .finish()
    }
}

/// Wo hochgeladene Dateien liegen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageBackend {
    /// Lokales Dateisystem unter `storage_dir` (Standard).
    Local,
    /// S3-kompatibler Objektspeicher (Garage, MinIO, AWS S3, …).
    S3(S3Config),
}

/// Zugang zu einem S3-kompatiblen Bucket.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Endpunkt, z. B. `http://garage:3900`; `None` = AWS S3 der Region.
    pub endpoint: Option<String>,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Pfad-Stil (`{endpoint}/{bucket}/{key}`) statt virtueller Hosts (`{bucket}.{endpoint}/{key}`).
    /// Garage und MinIO brauchen meist den Pfad-Stil.
    pub path_style: bool,
    /// Optionales Schlüssel-Präfix im Bucket, z. B. `lykos/attachments`.
    pub prefix: Option<String>,
}

impl std::fmt::Debug for S3Config {
    /// Ohne Secret – die Konfiguration landet sonst womöglich im Log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"***")
            .field("path_style", &self.path_style)
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl StorageBackend {
    /// Liest `LYKOS_STORAGE` und – für `s3` – die `LYKOS_S3_*`-Variablen über `get`
    /// (in Tests ohne echte Umgebungsvariablen aufrufbar).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let kind = get("LYKOS_STORAGE").unwrap_or_else(|| "local".into()).to_ascii_lowercase();
        match kind.as_str() {
            "local" => Ok(Self::Local),
            "s3" => {
                let require =
                    |name: &str| get(name).with_context(|| format!("{name} fehlt (LYKOS_STORAGE=s3)"));
                let endpoint = get("LYKOS_S3_ENDPOINT").map(|e| e.trim_end_matches('/').to_owned());
                if let Some(e) = &endpoint
                    && !(e.starts_with("http://") || e.starts_with("https://"))
                {
                    anyhow::bail!("LYKOS_S3_ENDPOINT muss mit http:// oder https:// beginnen");
                }
                let path_style = match get("LYKOS_S3_PATH_STYLE").map(|v| v.to_ascii_lowercase()).as_deref() {
                    None | Some("true" | "1" | "yes") => true,
                    Some("false" | "0" | "no") => false,
                    Some(_) => anyhow::bail!("LYKOS_S3_PATH_STYLE muss true oder false sein"),
                };
                let prefix =
                    get("LYKOS_S3_PREFIX").map(|p| p.trim_matches('/').to_owned()).filter(|p| !p.is_empty());
                Ok(Self::S3(S3Config {
                    endpoint,
                    bucket: require("LYKOS_S3_BUCKET")?,
                    region: get("LYKOS_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                    access_key_id: require("LYKOS_S3_ACCESS_KEY_ID")?,
                    secret_access_key: require("LYKOS_S3_SECRET_ACCESS_KEY")?,
                    path_style,
                    prefix,
                }))
            }
            other => anyhow::bail!("LYKOS_STORAGE muss local oder s3 sein, nicht {other:?}"),
        }
    }
}

/// Standard-Kontingent pro Nutzer für Anhänge (MB).
pub const DEFAULT_USER_QUOTA_MB: u64 = 2048;

/// Origins, unter denen der Tauri-Client auf den Plattformen läuft.
const TAURI_ORIGINS: &str = "tauri://localhost,http://tauri.localhost,https://tauri.localhost";

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env::var("LYKOS_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8080".into())
            .parse()
            .context("LYKOS_BIND ist keine gültige Socket-Adresse")?;

        let instance_name = env::var("LYKOS_INSTANCE_NAME").unwrap_or_else(|_| "Lykos".into());
        let source_url = env::var("LYKOS_SOURCE_URL")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_SOURCE_URL.into());
        anyhow::ensure!(
            source_url.starts_with("https://") || source_url.starts_with("http://"),
            "LYKOS_SOURCE_URL muss mit http:// oder https:// beginnen"
        );

        let database_url = env::var("LYKOS_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .context("LYKOS_DATABASE_URL fehlt")?;

        let public_url = env::var("LYKOS_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8080".into());

        let web_dir = env::var("LYKOS_WEB_DIR").ok().filter(|s| !s.is_empty()).map(PathBuf::from);

        let cors_origins = env::var("LYKOS_CORS_ORIGINS")
            .unwrap_or_else(|_| TAURI_ORIGINS.into())
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        let trust_proxy = env::var("LYKOS_TRUST_PROXY")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(false);
        let trusted_proxies = env::var("LYKOS_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<IpNet>().with_context(|| format!("LYKOS_TRUSTED_PROXIES: ungültige Adresse „{s}“"))
            })
            .collect::<Result<Vec<_>>>()?;

        let storage_dir = env::var("LYKOS_STORAGE_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "./data/files".into());
        let storage_dir = PathBuf::from(storage_dir);
        let storage = StorageBackend::from_lookup(|name| env::var(name).ok())?;

        let max_upload_mb: u64 = env::var("LYKOS_MAX_UPLOAD_MB")
            .unwrap_or_else(|_| "100".into())
            .parse()
            .ok()
            .filter(|mb| *mb > 0)
            .context("LYKOS_MAX_UPLOAD_MB muss eine positive Ganzzahl sein")?;
        let max_upload_bytes = max_upload_mb * 1024 * 1024;

        let user_quota_mb: u64 = env::var("LYKOS_USER_QUOTA_MB")
            .unwrap_or_else(|_| DEFAULT_USER_QUOTA_MB.to_string())
            .trim()
            .parse()
            .context("LYKOS_USER_QUOTA_MB muss eine Ganzzahl ≥ 0 sein (0 = unbegrenzt)")?;
        let user_quota_bytes = (user_quota_mb > 0).then(|| user_quota_mb.saturating_mul(1024 * 1024));

        let livekit = LiveKitConfig::from_env()?;

        let desktop_releases_url = env::var("LYKOS_DESKTOP_RELEASES_URL")
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        if let Some(url) = &desktop_releases_url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            anyhow::bail!("LYKOS_DESKTOP_RELEASES_URL muss eine http(s)-URL sein");
        }

        let feedback_gitea = FeedbackGiteaConfig::from_lookup(|name| env::var(name).ok())?;

        Ok(Self {
            bind,
            instance_name,
            source_url,
            database_url,
            public_url,
            web_dir,
            cors_origins,
            trust_proxy,
            trusted_proxies,
            storage_dir,
            storage,
            max_upload_bytes,
            user_quota_bytes,
            livekit,
            desktop_releases_url,
            mail: MailConfig::from_lookup(|name| env::var(name).ok())?,
            oidc_providers: OidcProviderConfig::from_lookup(|name| env::var(name).ok())?,
            call_ring_secs: DEFAULT_CALL_RING_SECS,
            feedback_gitea,
        })
    }
}

impl LiveKitConfig {
    /// Voice ist nur aktiv, wenn öffentliche URL, API-Key und Secret gesetzt sind.
    fn from_env() -> Result<Option<Self>> {
        let get = |name: &str| env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let (Some(url), Some(api_key), Some(api_secret)) =
            (get("LYKOS_LIVEKIT_URL"), get("LIVEKIT_API_KEY"), get("LIVEKIT_API_SECRET"))
        else {
            return Ok(None);
        };
        let api_url = get("LYKOS_LIVEKIT_API_URL").unwrap_or_else(|| "http://127.0.0.1:7880".into());
        if !api_url.starts_with("http://") {
            anyhow::bail!("LYKOS_LIVEKIT_API_URL muss eine interne http://-URL sein");
        }
        Ok(Some(Self { url, api_url: api_url.trim_end_matches('/').to_owned(), api_key, api_secret }))
    }

    /// Test-Zugangsdaten; die API-URL zeigt ins Leere (Aufrufe scheitern still).
    pub fn for_tests() -> Self {
        Self {
            url: "ws://livekit.test".into(),
            api_url: "http://127.0.0.1:9".into(),
            api_key: "test-key".into(),
            api_secret: "test-secret-mit-mindestens-32-zeichen".into(),
        }
    }
}

impl Config {
    /// Konfiguration für Integrationstests (keine Umgebungsvariablen nötig).
    pub fn for_tests() -> Self {
        Self {
            bind: "127.0.0.1:0".parse().expect("gültige Adresse"),
            instance_name: "Test".into(),
            source_url: DEFAULT_SOURCE_URL.into(),
            database_url: String::new(),
            public_url: "http://localhost".into(),
            web_dir: None,
            cors_origins: vec![],
            trust_proxy: false,
            trusted_proxies: vec![],
            // Eigenes Verzeichnis pro Test; wird erst beim ersten Upload angelegt.
            storage_dir: std::env::temp_dir().join("lykos-tests").join(uuid::Uuid::now_v7().to_string()),
            storage: StorageBackend::Local,
            max_upload_bytes: 100 * 1024 * 1024,
            user_quota_bytes: Some(DEFAULT_USER_QUOTA_MB * 1024 * 1024),
            livekit: None,
            desktop_releases_url: None,
            mail: MailConfig::Disabled,
            oidc_providers: vec![],
            feedback_gitea: None,
            call_ring_secs: DEFAULT_CALL_RING_SECS,
        }
    }

    /// Wie `for_tests`, aber mit aktiviertem Voice (Test-Keys, siehe `LiveKitConfig::for_tests`).
    pub fn for_tests_with_voice() -> Self {
        Self { livekit: Some(LiveKitConfig::for_tests()), ..Self::for_tests() }
    }
}

/// IP-Adresse oder Netz in CIDR-Schreibweise (`10.0.0.5`, `10.0.0.0/8`, `fd00::/8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// Liegt `ip` in diesem Netz? IPv4-gemappte IPv6-Adressen (`::ffff:a.b.c.d`) zählen als IPv4.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(self.prefix)).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl std::str::FromStr for IpNet {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr: IpAddr = addr.parse::<IpAddr>()?.to_canonical();
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).context("ungültige Präfixlänge")?,
            None => max,
        };
        Ok(Self { addr, prefix })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn ip_net_matches_addresses_and_ranges() {
        let single: IpNet = "10.0.0.5".parse().unwrap();
        assert!(single.contains("10.0.0.5".parse().unwrap()));
        assert!(single.contains("::ffff:10.0.0.5".parse().unwrap()), "IPv4-gemappt zählt");
        assert!(!single.contains("10.0.0.6".parse().unwrap()));

        let lan: IpNet = "10.0.0.0/24".parse().unwrap();
        assert!(lan.contains("10.0.0.200".parse().unwrap()));
        assert!(!lan.contains("10.0.1.1".parse().unwrap()));

        let v6: IpNet = "fd00::/8".parse().unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
        assert!(!v6.contains("2001:db8::1".parse().unwrap()));
        assert!(!v6.contains("10.0.0.1".parse().unwrap()));

        let all: IpNet = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains("8.8.8.8".parse().unwrap()));

        assert!("10.0.0.0/33".parse::<IpNet>().is_err());
        assert!("kein-netz".parse::<IpNet>().is_err());
    }

    fn backend(vars: &[(&str, &str)]) -> Result<StorageBackend> {
        let map: HashMap<String, String> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        StorageBackend::from_lookup(|name| map.get(name).cloned())
    }

    const S3_MIN: &[(&str, &str)] = &[
        ("LYKOS_STORAGE", "s3"),
        ("LYKOS_S3_BUCKET", "lykos"),
        ("LYKOS_S3_ACCESS_KEY_ID", "GK123"),
        ("LYKOS_S3_SECRET_ACCESS_KEY", "geheim"),
    ];

    #[test]
    fn feedback_gitea_needs_url_and_token() {
        let lookup = |vars: &[(&str, &str)]| {
            let map: HashMap<String, String> =
                vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
            FeedbackGiteaConfig::from_lookup(|name| map.get(name).cloned())
        };
        assert_eq!(lookup(&[]).unwrap(), None);
        let cfg = lookup(&[
            ("LYKOS_FEEDBACK_GITEA_URL", "https://git.example/api/v1/repos/a/b/"),
            ("LYKOS_FEEDBACK_GITEA_TOKEN", "geheim"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(cfg.api_url, "https://git.example/api/v1/repos/a/b");
        assert!(!format!("{cfg:?}").contains("geheim"));
        assert!(lookup(&[("LYKOS_FEEDBACK_GITEA_URL", "https://git.example")]).is_err());
        assert!(lookup(&[("LYKOS_FEEDBACK_GITEA_TOKEN", "x")]).is_err());
        assert!(
            lookup(&[("LYKOS_FEEDBACK_GITEA_URL", "git.example"), ("LYKOS_FEEDBACK_GITEA_TOKEN", "x")])
                .is_err()
        );
    }

    #[test]
    fn storage_defaults_to_local() {
        assert_eq!(backend(&[]).unwrap(), StorageBackend::Local);
        assert_eq!(backend(&[("LYKOS_STORAGE", " LOCAL ")]).unwrap(), StorageBackend::Local);
        assert!(backend(&[("LYKOS_STORAGE", "ftp")]).is_err());
    }

    #[test]
    fn s3_minimal_and_defaults() {
        let StorageBackend::S3(s3) = backend(S3_MIN).unwrap() else { panic!("kein S3") };
        assert_eq!(s3.bucket, "lykos");
        assert_eq!(s3.region, "us-east-1");
        assert_eq!(s3.endpoint, None);
        assert!(s3.path_style);
        assert_eq!(s3.prefix, None);
        // Secret taucht im Debug-Text nicht auf.
        assert!(!format!("{s3:?}").contains("geheim"));
    }

    #[test]
    fn livekit_debug_hides_secret() {
        let config = LiveKitConfig::for_tests();
        let text = format!("{config:?}");
        assert!(!text.contains(&config.api_secret), "{text}");
        assert!(text.contains("test-key"));
    }

    #[test]
    fn s3_full() {
        let mut vars = S3_MIN.to_vec();
        vars.extend([
            ("LYKOS_S3_ENDPOINT", "http://garage:3900/"),
            ("LYKOS_S3_REGION", "garage"),
            ("LYKOS_S3_PATH_STYLE", "false"),
            ("LYKOS_S3_PREFIX", "/lykos/anhaenge/"),
        ]);
        let StorageBackend::S3(s3) = backend(&vars).unwrap() else { panic!("kein S3") };
        assert_eq!(s3.endpoint.as_deref(), Some("http://garage:3900"));
        assert_eq!(s3.region, "garage");
        assert!(!s3.path_style);
        assert_eq!(s3.prefix.as_deref(), Some("lykos/anhaenge"));
    }

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn mail_config() {
        assert_eq!(MailConfig::from_lookup(lookup(&[])).unwrap(), MailConfig::Disabled);
        assert_eq!(MailConfig::from_lookup(lookup(&[("LYKOS_MAIL", "log")])).unwrap(), MailConfig::Log);
        // SMTP ohne Absender ist ein Fehler.
        assert!(MailConfig::from_lookup(lookup(&[("LYKOS_SMTP_URL", "smtps://a:b@mail.example")])).is_err());
        let MailConfig::Smtp(smtp) = MailConfig::from_lookup(lookup(&[
            ("LYKOS_SMTP_URL", "smtps://lykos%40example.org:g%3Aheim@mail.example.org:465"),
            ("LYKOS_MAIL_FROM", "lykos@example.org"),
        ]))
        .unwrap() else {
            panic!("kein SMTP")
        };
        assert_eq!(smtp.host, "mail.example.org");
        assert_eq!(smtp.port, Some(465));
        assert_eq!(smtp.tls, SmtpTls::Tls);
        assert_eq!(smtp.username.as_deref(), Some("lykos@example.org"));
        assert_eq!(smtp.password.as_deref(), Some("g:heim"));
        assert!(!format!("{smtp:?}").contains("g:heim"));

        let MailConfig::Smtp(smtp) = MailConfig::from_lookup(lookup(&[
            ("LYKOS_SMTP_HOST", "relay.local"),
            ("LYKOS_SMTP_TLS", "starttls"),
            ("LYKOS_SMTP_PORT", "587"),
            ("LYKOS_MAIL_FROM", "lykos@example.org"),
        ]))
        .unwrap() else {
            panic!("kein SMTP")
        };
        assert_eq!((smtp.host.as_str(), smtp.port, smtp.tls), ("relay.local", Some(587), SmtpTls::StartTls));
        assert_eq!(smtp.username, None);
        assert!(MailConfig::from_lookup(lookup(&[("LYKOS_MAIL", "brieftaube")])).is_err());
        assert!(
            MailConfig::from_lookup(lookup(&[("LYKOS_SMTP_URL", "http://x"), ("LYKOS_MAIL_FROM", "a@b.de")]))
                .is_err()
        );
    }

    #[test]
    fn smtp_parts_with_special_characters() {
        // Getrennte Variablen: Sonderzeichen im Passwort brauchen keine URL-Kodierung.
        let MailConfig::Smtp(smtp) = MailConfig::from_lookup(lookup(&[
            ("LYKOS_SMTP_HOST", "smtp.example.org"),
            ("LYKOS_SMTP_PORT", "465"),
            ("LYKOS_SMTP_TLS", "implicit"),
            ("LYKOS_SMTP_USER", "lykos@example.org"),
            ("LYKOS_SMTP_PASSWORD", "g3h!m:@%20#"),
            ("LYKOS_MAIL_FROM", "lykos@example.org"),
        ]))
        .unwrap() else {
            panic!("kein SMTP")
        };
        assert_eq!(smtp.tls, SmtpTls::Tls);
        assert_eq!(smtp.password.as_deref(), Some("g3h!m:@%20#"));
        assert!(
            MailConfig::from_lookup(lookup(&[
                ("LYKOS_SMTP_HOST", "x.example.org"),
                ("LYKOS_SMTP_TLS", "vielleicht"),
                ("LYKOS_MAIL_FROM", "a@b.de"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn oauth_providers() {
        assert!(OidcProviderConfig::from_lookup(lookup(&[])).unwrap().is_empty());
        let providers = OidcProviderConfig::from_lookup(lookup(&[
            ("LYKOS_OAUTH_PROVIDERS", "Discord, github,google, firma, my-idp"),
            ("LYKOS_OAUTH_DISCORD_CLIENT_ID", "123"),
            ("LYKOS_OAUTH_DISCORD_CLIENT_SECRET", "geheim"),
            ("LYKOS_OAUTH_GITHUB_CLIENT_ID", "gh"),
            ("LYKOS_OAUTH_GITHUB_CLIENT_SECRET", "gh-geheim"),
            ("LYKOS_OAUTH_GOOGLE_CLIENT_ID", "g"),
            ("LYKOS_OAUTH_FIRMA_ISSUER", "https://sso.example.org/realms/lykos/"),
            ("LYKOS_OAUTH_FIRMA_CLIENT_ID", "lykos"),
            ("LYKOS_OAUTH_FIRMA_LABEL", "Firmen-Login"),
            ("LYKOS_OAUTH_MY_IDP_AUTHORIZE_URL", "http://localhost:9000/auth"),
            ("LYKOS_OAUTH_MY_IDP_TOKEN_URL", "http://localhost:9000/token"),
            ("LYKOS_OAUTH_MY_IDP_USERINFO_URL", "http://localhost:9000/me"),
            ("LYKOS_OAUTH_MY_IDP_SUBJECT_FIELD", "uid"),
            ("LYKOS_OAUTH_MY_IDP_CLIENT_ID", "abc"),
        ]))
        .unwrap();
        let ids: Vec<_> = providers.iter().map(|p| (p.id.as_str(), p.label.as_str())).collect();
        assert_eq!(
            ids,
            [
                ("discord", "Discord"),
                ("github", "GitHub"),
                ("google", "Google"),
                ("firma", "Firmen-Login"),
                ("my-idp", "my-idp")
            ]
        );
        let AuthProtocol::OAuth2(discord) = &providers[0].protocol else { panic!("Discord ist OAuth2") };
        assert_eq!(discord.userinfo_url, "https://discord.com/api/users/@me");
        assert_eq!(discord.subject_field, "id");
        assert_eq!(providers[0].scopes, ["identify"]);
        assert!(!format!("{:?}", providers[0]).contains("geheim"));
        let AuthProtocol::OAuth2(github) = &providers[1].protocol else { panic!("GitHub ist OAuth2") };
        assert_eq!(github.email_field, None);
        assert!(providers[1].scopes.is_empty());
        assert_eq!(
            providers[2].protocol,
            AuthProtocol::Oidc { issuer: "https://accounts.google.com".into() }
        );
        assert_eq!(providers[2].client_secret, None);
        assert_eq!(
            providers[3].protocol,
            AuthProtocol::Oidc { issuer: "https://sso.example.org/realms/lykos".into() }
        );
        assert_eq!(providers[3].scopes, ["openid", "email"]);
        let AuthProtocol::OAuth2(custom) = &providers[4].protocol else { panic!("OAuth2") };
        assert_eq!(custom.subject_field, "uid");

        // http nur lokal, Pflichtfelder, gültige Kennungen.
        let base = [("LYKOS_OAUTH_PROVIDERS", "x"), ("LYKOS_OAUTH_X_CLIENT_ID", "c")];
        let with = |issuer: &str| {
            let mut v = base.to_vec();
            v.push(("LYKOS_OAUTH_X_ISSUER", issuer));
            OidcProviderConfig::from_lookup(lookup(&v))
        };
        assert!(with("http://sso.example.org").is_err());
        assert!(with("http://localhost.evil.example").is_err());
        assert!(with("https://sso.example.org").is_ok());
        assert!(OidcProviderConfig::from_lookup(lookup(&base)).is_err(), "weder Issuer noch Endpunkte");
        assert!(OidcProviderConfig::from_lookup(lookup(&[("LYKOS_OAUTH_PROVIDERS", "discord")])).is_err());
        assert!(OidcProviderConfig::from_lookup(lookup(&[("LYKOS_OAUTH_PROVIDERS", "a/b")])).is_err());
    }

    #[test]
    fn s3_rejects_missing_or_invalid_values() {
        for missing in ["LYKOS_S3_BUCKET", "LYKOS_S3_ACCESS_KEY_ID", "LYKOS_S3_SECRET_ACCESS_KEY"] {
            let vars: Vec<_> = S3_MIN.iter().copied().filter(|(k, _)| *k != missing).collect();
            let err = backend(&vars).unwrap_err().to_string();
            assert!(err.contains(missing), "{err}");
        }
        let mut vars = S3_MIN.to_vec();
        vars.push(("LYKOS_S3_ENDPOINT", "garage:3900"));
        assert!(backend(&vars).is_err());
        let mut vars = S3_MIN.to_vec();
        vars.push(("LYKOS_S3_PATH_STYLE", "vielleicht"));
        assert!(backend(&vars).is_err());
    }
}
