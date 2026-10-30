//! Kompression und Cache-Header: Web-Client (gehashte Assets, index.html), API und Anhänge.

mod common;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use common::{TestApp, s};
use http_body_util::BodyExt;
use lykos_server::config::Config;
use serde_json::json;
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;

const BOUNDARY: &str = "lykos-test-boundary-7f3a";

struct Env {
    app: TestApp,
    _web: TempDir,
    _storage: TempDir,
}

fn env(db: PgPool) -> Env {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<!doctype html><title>Lykos</title>".repeat(20)).unwrap();
    std::fs::create_dir(web.path().join("assets")).unwrap();
    std::fs::write(web.path().join("assets/index-Ab12Cd34.js"), "console.log('lykos');\n".repeat(200))
        .unwrap();
    let storage = tempfile::tempdir().unwrap();
    let mut config = Config::for_tests();
    config.web_dir = Some(web.path().to_path_buf());
    config.storage_dir = storage.path().to_path_buf();
    Env { app: TestApp::with_config(db, config), _web: web, _storage: storage }
}

async fn get(
    app: &TestApp,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        req = req.header(name, *value);
    }
    let res = app.router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn static_files_are_cached_and_compressed(db: PgPool) {
    let env = env(db);
    let gzip = [(header::ACCEPT_ENCODING, "gzip")];

    // Gehashte Assets: ewig cachebar, komprimiert.
    let (status, h, body) = get(&env.app, "/assets/index-Ab12Cd34.js", &gzip).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
    assert_eq!(h[header::CONTENT_ENCODING], "gzip");
    assert!(body.len() < 4400, "nicht komprimiert: {} Bytes", body.len());

    // Brotli wird bevorzugt, wenn der Client es kann.
    let (_, h, _) =
        get(&env.app, "/assets/index-Ab12Cd34.js", &[(header::ACCEPT_ENCODING, "gzip, br")]).await;
    assert_eq!(h[header::CONTENT_ENCODING], "br");

    // index.html und SPA-Fallback: immer neu prüfen – auch für fehlende Assets.
    for uri in ["/", "/index.html", "/channels/irgendwas", "/assets/gibt-es-nicht.js"] {
        let (status, h, _) = get(&env.app, uri, &gzip).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(h[header::CACHE_CONTROL], "no-cache", "{uri}");
        assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"), "{uri}");
    }

    // Range auf ein Asset bleibt unkomprimiert.
    let (status, h, body) = get(
        &env.app,
        "/assets/index-Ab12Cd34.js",
        &[(header::ACCEPT_ENCODING, "gzip"), (header::RANGE, "bytes=0-6")],
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert!(h.get(header::CONTENT_ENCODING).is_none());
    assert_eq!(body, b"console");

    // Ohne Accept-Encoding: unkomprimiert.
    let (_, h, _) = get(&env.app, "/assets/index-Ab12Cd34.js", &[]).await;
    assert!(h.get(header::CONTENT_ENCODING).is_none());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn api_is_compressed_but_attachments_are_not(db: PgPool) {
    let env = env(db);
    let alex = env.app.user("alex", None).await;
    let community = env.app.community(&alex, "Rudel").await;
    let auth = format!("Bearer {alex}");

    // API-JSON wird komprimiert und bekommt keinen Asset-Cache-Header.
    let uri = format!("/api/communities/{}", community.id);
    let (status, h, _) =
        get(&env.app, &uri, &[(header::ACCEPT_ENCODING, "gzip"), (header::AUTHORIZATION, &auth)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_ENCODING], "gzip");
    assert_ne!(h.get(header::CACHE_CONTROL).map(|v| v.to_str().unwrap()), Some("no-cache"));

    // Anhang (Text, also eigentlich gut komprimierbar): nie komprimiert, Range funktioniert.
    let text = "Zeile mit Text\n".repeat(100);
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notiz.txt\"\r\n\
         Content-Type: text/plain\r\n\r\n{text}\r\n--{BOUNDARY}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{}/attachments", community.text_channel))
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
        .header(header::AUTHORIZATION, &auth)
        .body(Body::from(body))
        .unwrap();
    let (status, attachment) = env.app.send(req).await;
    assert_eq!(status, StatusCode::CREATED, "{attachment}");
    let (status, _) = env
        .app
        .call(
            "POST",
            &format!("/channels/{}/messages", community.text_channel),
            Some(&alex),
            Some(json!({ "content": "", "attachment_ids": [s(&attachment["id"])] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let url = s(&attachment["url"]);
    let (status, h, bytes) =
        get(&env.app, url, &[(header::ACCEPT_ENCODING, "gzip, br"), (header::AUTHORIZATION, &auth)]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(h.get(header::CONTENT_ENCODING).is_none());
    assert_eq!(bytes, text.as_bytes());
    let (status, h, bytes) = get(
        &env.app,
        url,
        &[(header::ACCEPT_ENCODING, "gzip"), (header::AUTHORIZATION, &auth), (header::RANGE, "bytes=0-4")],
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert!(h.get(header::CONTENT_ENCODING).is_none());
    assert_eq!(bytes, b"Zeile");
}
