//! Integrationstests für die Desktop-Endpunkte (Download-Seite, Tauri-Update-Manifest).
//! Die Gitea-Release-API wird durch einen kleinen Mock-Server ersetzt.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use common::TestApp;
use lykos_server::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Clone)]
struct Mock {
    base: String,
    /// Aufrufe der Release-Liste.
    hits: Arc<AtomicUsize>,
    /// Release-Liste antwortet mit 500.
    broken: bool,
    /// v0.3.0 mit `.sig`-Datei.
    signed: bool,
}

async fn mock_gitea(broken: bool, signed: bool) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mock = Mock { base, hits: Arc::default(), broken, signed };

    async fn releases(State(m): State<Mock>) -> Result<Json<Value>, StatusCode> {
        m.hits.fetch_add(1, Ordering::SeqCst);
        if m.broken {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let asset = |name: &str, size: u64| json!({ "name": name, "size": size, "browser_download_url": format!("{}/dl/{name}", m.base) });
        let exe = "Lykos-0.3.0-windows-x64-setup.exe";
        let mut assets = vec![asset(exe, 2_400_000), asset(&format!("{exe}.sha256"), 101)];
        if m.signed {
            assets.push(asset(&format!("{exe}.sig"), 400));
        }
        Ok(Json(json!([
            // Entwurf: wird übersprungen.
            { "tag_name": "v0.4.0", "draft": true, "prerelease": false, "body": "",
              "assets": [asset("Lykos-0.4.0-windows-x64-setup.exe", 1)] },
            // Ohne Installer: wird übersprungen.
            { "tag_name": "v0.3.1", "draft": false, "prerelease": true, "body": "nur Doku", "assets": [] },
            { "tag_name": "v0.3.0", "draft": false, "prerelease": true, "body": "## Neu\n- Updates",
              "published_at": "2026-11-01T10:00:00Z",
              "html_url": format!("{}/ZeasyAlex/Lykos-Releases/releases/tag/v0.3.0", m.base),
              "assets": assets },
            { "tag_name": "v0.2.0", "draft": false, "prerelease": true, "body": "",
              "assets": [asset("Lykos-0.2.0-windows-x64-setup.exe", 2)] },
        ])))
    }
    async fn sha(State(_): State<Mock>) -> String {
        format!("{SHA}  Lykos-0.3.0-windows-x64-setup.exe\n")
    }
    async fn sig(State(_): State<Mock>) -> &'static str {
        "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==\n"
    }

    let router = Router::new()
        .route("/api/v1/repos/ZeasyAlex/Lykos-Releases/releases", get(releases))
        .route("/dl/Lykos-0.3.0-windows-x64-setup.exe.sha256", get(sha))
        .route("/dl/Lykos-0.3.0-windows-x64-setup.exe.sig", get(sig))
        .with_state(mock.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    mock
}

fn app_for(db: PgPool, mock: &Mock) -> TestApp {
    let mut config = Config::for_tests();
    config.desktop_releases_url =
        Some(format!("{}/api/v1/repos/ZeasyAlex/Lykos-Releases/releases", mock.base));
    TestApp::with_config(db, config)
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn desktop_endpoints_without_config(db: PgPool) {
    let app = TestApp::new(db);
    let (status, body) = app.call("GET", "/desktop/latest", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "no_release");
    let (status, body) = app.call("GET", "/desktop/latest.json", None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "kein Update: {body}");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn latest_release_and_update_manifest(db: PgPool) {
    let mock = mock_gitea(false, true).await;
    let app = app_for(db, &mock);

    // Öffentlich, ohne Anmeldung.
    let (status, body) = app.call("GET", "/desktop/latest", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let exe_url = format!("{}/dl/Lykos-0.3.0-windows-x64-setup.exe", mock.base);
    assert_eq!(
        body,
        json!({
            "version": "0.3.0",
            "notes": "## Neu\n- Updates",
            "published_at": "2026-11-01T10:00:00Z",
            "release_url": format!("{}/ZeasyAlex/Lykos-Releases/releases/tag/v0.3.0", mock.base),
            "file_name": "Lykos-0.3.0-windows-x64-setup.exe",
            "download_url": exe_url,
            "size": 2_400_000,
            "sha256": SHA,
        })
    );

    let (status, manifest) = app.call("GET", "/desktop/latest.json", None, None).await;
    assert_eq!(status, StatusCode::OK, "{manifest}");
    assert_eq!(
        manifest,
        json!({
            "version": "0.3.0",
            "notes": "## Neu\n- Updates",
            "pub_date": "2026-11-01T10:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==",
                    "url": exe_url,
                },
            },
        })
    );

    // Gecacht: die Release-API wurde nur einmal gefragt.
    app.call("GET", "/desktop/latest", None, None).await;
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn unsigned_release_has_no_update(db: PgPool) {
    let mock = mock_gitea(false, false).await;
    let app = app_for(db, &mock);
    let (status, body) = app.call("GET", "/desktop/latest", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"], "0.3.0");
    let (status, _) = app.call("GET", "/desktop/latest.json", None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn release_api_down(db: PgPool) {
    let mock = mock_gitea(true, true).await;
    let app = app_for(db, &mock);
    let (status, body) = app.call("GET", "/desktop/latest", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "desktop_unavailable");
    let (status, _) = app.call("GET", "/desktop/latest.json", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    // Fehler werden kurz gecacht – kein Sturm auf die Release-API.
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
}
