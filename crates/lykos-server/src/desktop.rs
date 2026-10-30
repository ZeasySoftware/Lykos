//! Desktop-Releases: neuester Windows-Installer aus dem öffentlichen Release-Repo (Gitea- oder GitHub-API;
//! beide liefern dieselben Felder).
//!
//! Quelle ist die Release-Liste (`LYKOS_DESKTOP_RELEASES_URL`). Verwendet wird das neueste
//! veröffentlichte Release (keine Entwürfe; Vorabversionen zählen mit, der Pilot veröffentlicht nur
//! solche) mit einem Asset `*-setup.exe`. Daneben liegen optional `<installer>.sha256`
//! (`sha256sum`-Format) und `<installer>.sig` (Tauri-Updater-Signatur). Daraus entstehen die Daten
//! der Download-Seite und das statische Tauri-Update-Manifest (`latest.json`).
//!
//! Das Ergebnis wird im Speicher gecacht (`CACHE_TTL`). Ist die Release-API nicht erreichbar, bleibt der
//! letzte Stand gültig und es wird nach `RETRY_AFTER` erneut versucht.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// Wie lange ein erfolgreich geladener Stand gilt.
pub const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// Nach einem Fehler: so lange bis zum nächsten Versuch.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Wie viele Releases durchsucht werden.
const RELEASES_LIMIT: &str = "10";
/// Obergrenze für `.sha256`/`.sig`-Dateien.
const MAX_SIDE_FILE: usize = 16 * 1024;
/// Obergrenze für die Antwort der Release-Liste (JSON, wenige Releases).
const MAX_RELEASE_LIST: usize = 2 * 1024 * 1024;

/// Neuester Desktop-Installer, wie ihn die Download-Seite anzeigt (`GET /api/desktop/latest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopRelease {
    /// Semver ohne führendes `v`, z. B. `0.3.0`.
    pub version: String,
    /// Release-Notizen (Markdown).
    pub notes: String,
    /// Veröffentlichungszeitpunkt (RFC 3339), falls bekannt.
    pub published_at: Option<String>,
    /// Release-Seite im Repo.
    pub release_url: String,
    pub file_name: String,
    pub download_url: String,
    pub size: u64,
    /// SHA-256 des Installers (hex, klein), falls eine `.sha256`-Datei dabei ist.
    pub sha256: Option<String>,
    /// Tauri-Updater-Signatur (Inhalt der `.sig`-Datei); ohne sie gibt es kein Update-Manifest.
    #[serde(skip)]
    pub signature: Option<String>,
}

impl DesktopRelease {
    /// Statisches Tauri-Update-Manifest; `None` ohne Signatur (dann „kein Update“).
    pub fn update_manifest(&self) -> Option<Value> {
        let signature = self.signature.as_ref()?;
        Some(json!({
            "version": self.version,
            "notes": self.notes,
            "pub_date": self.published_at,
            "platforms": {
                "windows-x86_64": { "signature": signature, "url": self.download_url },
            },
        }))
    }
}

#[derive(Debug, Deserialize)]
struct GiteaRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GiteaAsset>,
}

#[derive(Debug, Deserialize)]
struct GiteaAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

enum Cached {
    /// Erfolgreich geladen (`None` = kein passendes Release).
    Loaded(Option<Arc<DesktopRelease>>),
    /// Laden fehlgeschlagen, noch nie ein Stand vorhanden.
    Failed,
}

/// Cache für das neueste Desktop-Release (ein Eintrag, Abruf serialisiert).
#[derive(Default)]
pub struct DesktopReleases {
    cache: Mutex<Option<(Instant, Cached)>>,
}

/// Fehler beim Abruf ohne verwertbaren alten Stand.
#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable;

impl DesktopReleases {
    /// Neuestes Release (gecacht). `Ok(None)` = kein passendes Release vorhanden.
    pub async fn latest(&self, releases_url: &str) -> Result<Option<Arc<DesktopRelease>>, Unavailable> {
        let mut cache = self.cache.lock().await;
        if let Some((fetched, cached)) = cache.as_ref() {
            let ttl = match cached {
                Cached::Loaded(_) => CACHE_TTL,
                Cached::Failed => RETRY_AFTER,
            };
            if fetched.elapsed() < ttl {
                return match cached {
                    Cached::Loaded(release) => Ok(release.clone()),
                    Cached::Failed => Err(Unavailable),
                };
            }
        }

        match fetch_latest(releases_url).await {
            Ok(release) => {
                let release = release.map(Arc::new);
                *cache = Some((Instant::now(), Cached::Loaded(release.clone())));
                Ok(release)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Desktop-Releases konnten nicht geladen werden");
                match cache.take() {
                    // Alten Stand weiter ausliefern, aber bald neu versuchen.
                    Some((_, Cached::Loaded(release))) => {
                        let retry =
                            Instant::now().checked_sub(CACHE_TTL - RETRY_AFTER).unwrap_or_else(Instant::now);
                        *cache = Some((retry, Cached::Loaded(release.clone())));
                        Ok(release)
                    }
                    _ => {
                        *cache = Some((Instant::now(), Cached::Failed));
                        Err(Unavailable)
                    }
                }
            }
        }
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::install_crypto_provider();
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("lykos-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP-Client")
    })
}

/// Version aus dem Tag (`v0.3.0` → `0.3.0`); nur einfache Semver-Formen.
fn version_from_tag(tag: &str) -> Option<String> {
    let v = tag.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let parts: Vec<&str> = core.split('.').collect();
    let ok = parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| v.to_owned())
}

/// Liegt `url` auf dem Host der Release-API? Nur dorthin folgt der Server. Einzige Ausnahme: GitHub liefert
/// die API unter `api.github.com`, die Release-Dateien aber unter `https://github.com` (von dort leitet
/// GitHub auf seinen Datei-Host weiter).
fn same_origin(base: &reqwest::Url, url: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(url).ok()?;
    let same = url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default();
    let github = base.host_str() == Some("api.github.com")
        && url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port_or_known_default() == Some(443);
    (same || github).then_some(url)
}

/// Antwort stückweise lesen und abbrechen, sobald sie `max` Bytes überschreitet – ein großer oder
/// endloser Body landet so nie komplett im Speicher (`Content-Length` allein ist nicht verlässlich).
pub(crate) async fn read_limited(mut res: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if res.content_length().is_some_and(|len| len > max as u64) {
        bail!("{}: Antwort zu groß", res.url());
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if body.len() + chunk.len() > max {
            bail!("{}: Antwort zu groß", res.url());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch_text(url: reqwest::Url) -> Result<String> {
    let res = http().get(url.clone()).send().await.with_context(|| format!("{url} nicht erreichbar"))?;
    if !res.status().is_success() {
        bail!("{url}: {}", res.status());
    }
    let bytes = read_limited(res, MAX_SIDE_FILE).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Erstes Token einer `sha256sum`-Zeile, falls es ein SHA-256 ist.
fn parse_sha256(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

async fn fetch_latest(releases_url: &str) -> Result<Option<DesktopRelease>> {
    let base = reqwest::Url::parse(releases_url).context("LYKOS_DESKTOP_RELEASES_URL ungültig")?;
    let mut list_url = base.clone();
    // `limit` versteht Gitea, `per_page` GitHub – der jeweils andere Parameter wird ignoriert.
    list_url.query_pairs_mut().append_pair("limit", RELEASES_LIMIT).append_pair("per_page", RELEASES_LIMIT);
    let res = http()
        .get(list_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("Release-API nicht erreichbar")?;
    if !res.status().is_success() {
        bail!("Release-API: {}", res.status());
    }
    let body = read_limited(res, MAX_RELEASE_LIST).await?;
    let releases: Vec<GiteaRelease> =
        serde_json::from_slice(&body).context("Release-API: ungültige Antwort")?;

    for release in releases.into_iter().filter(|r| !r.draft) {
        let Some(version) = version_from_tag(&release.tag_name) else { continue };
        let Some(installer) = release.assets.iter().find(|a| a.name.ends_with("-setup.exe")) else {
            continue;
        };
        let Some(download_url) = same_origin(&base, &installer.browser_download_url) else {
            tracing::warn!(url = %installer.browser_download_url, "Installer liegt nicht auf dem Release-Host");
            continue;
        };
        let side_file = |suffix: &str| {
            let name = format!("{}{suffix}", installer.name);
            release
                .assets
                .iter()
                .find(|a| a.name == name)
                .and_then(|a| same_origin(&base, &a.browser_download_url))
        };
        let sha256 = match side_file(".sha256") {
            Some(url) => parse_sha256(&fetch_text(url).await?),
            None => None,
        };
        let signature = match side_file(".sig") {
            Some(url) => Some(fetch_text(url).await?.trim().to_owned()).filter(|s| !s.is_empty()),
            None => None,
        };
        return Ok(Some(DesktopRelease {
            version,
            notes: release.body.unwrap_or_default(),
            published_at: release.published_at,
            release_url: release.html_url,
            file_name: installer.name.clone(),
            download_url: download_url.to_string(),
            size: installer.size,
            sha256,
            signature,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oversized_bodies_are_rejected() {
        let res = |body: Vec<u8>| reqwest::Response::from(axum::http::Response::new(body));
        assert_eq!(read_limited(res(vec![b'a'; 10]), 16).await.unwrap().len(), 10);
        assert!(read_limited(res(vec![b'a'; 17]), 16).await.is_err());
        assert!(read_limited(res(vec![b'a'; MAX_SIDE_FILE + 1]), MAX_SIDE_FILE).await.is_err());
    }

    #[test]
    fn versions_from_tags() {
        assert_eq!(version_from_tag("v0.3.0").as_deref(), Some("0.3.0"));
        assert_eq!(version_from_tag("1.2.10").as_deref(), Some("1.2.10"));
        assert_eq!(version_from_tag("v1.0.0-beta.1").as_deref(), Some("1.0.0-beta.1"));
        assert_eq!(version_from_tag("v1.0"), None);
        assert_eq!(version_from_tag("latest"), None);
        assert_eq!(version_from_tag("v1.x.0"), None);
    }

    #[test]
    fn sha256_lines() {
        let hash = "a".repeat(64);
        assert_eq!(parse_sha256(&format!("{hash}  Lykos-0.3.0-windows-x64-setup.exe\n")), Some(hash.clone()));
        assert_eq!(parse_sha256(&hash.to_uppercase()), Some(hash));
        assert_eq!(parse_sha256("abc  datei.exe"), None);
        assert_eq!(parse_sha256(""), None);
    }

    #[test]
    fn origin_check() {
        let base = reqwest::Url::parse("https://git.example/api/v1/repos/a/b/releases").unwrap();
        assert!(same_origin(&base, "https://git.example/a/b/releases/download/v1/x.exe").is_some());
        assert!(same_origin(&base, "https://git.example:443/x.exe").is_some());
        assert!(same_origin(&base, "https://evil.example/x.exe").is_none());
        assert!(same_origin(&base, "http://git.example/x.exe").is_none());

        // GitHub: API auf api.github.com, Dateien auf github.com – sonst nichts.
        let gh = reqwest::Url::parse("https://api.github.com/repos/o/r/releases").unwrap();
        assert!(same_origin(&gh, "https://github.com/o/r/releases/download/v1/x.exe").is_some());
        assert!(same_origin(&gh, "https://api.github.com/x.exe").is_some());
        assert!(same_origin(&gh, "http://github.com/o/r/x.exe").is_none());
        assert!(same_origin(&gh, "https://github.com:8443/x.exe").is_none());
        assert!(same_origin(&gh, "https://evil.github.com/x.exe").is_none());
        assert!(same_origin(&gh, "https://objects.githubusercontent.com/x.exe").is_none());
        // Die Ausnahme gilt nur für die GitHub-API als Basis.
        assert!(same_origin(&base, "https://github.com/a/b/x.exe").is_none());
    }

    #[test]
    fn manifest_needs_signature() {
        let mut release = DesktopRelease {
            version: "0.3.0".into(),
            notes: "Neu".into(),
            published_at: Some("2026-10-29T14:58:45Z".into()),
            release_url: "https://git.example/r".into(),
            file_name: "Lykos-0.3.0-windows-x64-setup.exe".into(),
            download_url: "https://git.example/x.exe".into(),
            size: 1,
            sha256: None,
            signature: None,
        };
        assert_eq!(release.update_manifest(), None);
        release.signature = Some("c2ln".into());
        let manifest = release.update_manifest().unwrap();
        assert_eq!(manifest["version"], "0.3.0");
        assert_eq!(manifest["pub_date"], "2026-10-29T14:58:45Z");
        assert_eq!(manifest["platforms"]["windows-x86_64"]["signature"], "c2ln");
        assert_eq!(manifest["platforms"]["windows-x86_64"]["url"], "https://git.example/x.exe");
        // Die Signatur braucht nur das Update-Manifest.
        assert!(serde_json::to_value(&release).unwrap().get("signature").is_none());
    }
}
