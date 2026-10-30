//! Auslieferung des Web-Clients: SPA-Fallback und Sicherheits-Header (CSP, Framing, Referrer, HSTS).

mod common;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use common::TestApp;
use lykos_server::config::{Config, LiveKitConfig};
use sqlx::PgPool;

fn web_config(public_url: &str) -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<!doctype html><title>Lykos</title>").unwrap();
    let mut config = Config::for_tests();
    config.web_dir = Some(dir.path().to_owned());
    config.public_url = public_url.into();
    config.livekit =
        Some(LiveKitConfig { url: "wss://rtc.example.org".into(), ..LiveKitConfig::for_tests() });
    (config, dir)
}

async fn get(app: &TestApp, path: &str) -> (StatusCode, HeaderMap) {
    let (status, headers, _) = app.send_full(Request::builder().uri(path).body(Body::empty()).unwrap()).await;
    (status, headers)
}

fn header(headers: &HeaderMap, name: header::HeaderName) -> &str {
    headers.get(&name).unwrap_or_else(|| panic!("{name} fehlt")).to_str().unwrap()
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn web_client_has_security_headers(db: PgPool) {
    let (config, _dir) = web_config("https://chat.example.org");
    let app = TestApp::with_config(db, config);

    // Startseite, Download-Seite und Einladungen sind SPA-Routen (Fallback auf index.html).
    for path in ["/", "/download", "/invite/abc123", "/reset-password"] {
        let (status, headers) = get(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let csp = header(&headers, header::CONTENT_SECURITY_POLICY);
        for directive in [
            "default-src 'self'",
            "script-src 'self'",
            "object-src 'none'",
            "base-uri 'self'",
            "frame-ancestors 'none'",
            "form-action 'self'",
            "img-src 'self' data: blob:",
            "media-src 'self' blob:",
            "worker-src 'self' blob:",
            "style-src 'self' 'unsafe-inline'",
            "font-src 'self'",
        ] {
            assert!(csp.contains(directive), "{path}: {directive} fehlt in {csp}");
        }
        assert!(csp.contains(
            "connect-src 'self' wss://chat.example.org wss://rtc.example.org https://rtc.example.org"
        ));
        assert_eq!(header(&headers, header::X_FRAME_OPTIONS), "DENY");
        assert_eq!(header(&headers, header::REFERRER_POLICY), "no-referrer");
        assert_eq!(header(&headers, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(header(&headers, header::STRICT_TRANSPORT_SECURITY), "max-age=31536000");
    }

    // Auch die API bekommt die Header.
    let (status, headers) = get(&app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(header(&headers, header::X_FRAME_OPTIONS), "DENY");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn no_hsts_without_https(db: PgPool) {
    let (config, _dir) = web_config("http://localhost:8080");
    let app = TestApp::with_config(db, config);
    let (status, headers) = get(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::STRICT_TRANSPORT_SECURITY).is_none());
    assert!(header(&headers, header::CONTENT_SECURITY_POLICY).contains("ws://localhost:8080"));
}

/// Gateway: Browser-Verbindungen nur vom eigenen Web-Client und von den CORS-Origins (Desktop-App).
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_checks_origin(db: PgPool) {
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::HeaderValue};

    let mut config = Config::for_tests();
    config.public_url = "https://chat.example.org/".into();
    config.cors_origins = vec!["tauri://localhost".into()];
    let app = TestApp::with_config(db, config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let connect = async |origin: Option<&'static str>| {
        let mut req = format!("ws://{addr}/api/gateway").into_client_request().unwrap();
        if let Some(origin) = origin {
            req.headers_mut().insert("origin", HeaderValue::from_static(origin));
        }
        tokio_tungstenite::connect_async(req).await
    };
    let allowed =
        [None, Some("https://chat.example.org"), Some("HTTPS://Chat.Example.org"), Some("tauri://localhost")];
    for origin in allowed {
        assert!(connect(origin).await.is_ok(), "{origin:?} sollte erlaubt sein");
    }
    for origin in ["https://evil.example", "https://chat.example.org.evil.example", "null"] {
        match connect(Some(origin)).await {
            Err(tungstenite::Error::Http(res)) => assert_eq!(res.status(), StatusCode::FORBIDDEN, "{origin}"),
            other => panic!("{origin} hätte abgelehnt werden müssen: {other:?}"),
        }
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn web_client_login_gets_cookie_but_no_token(db: PgPool) {
    let (config, _dir) = web_config("https://chat.example.org");
    let app = TestApp::with_config(db, config);
    app.user("alex", None).await;

    let login = |origin: Option<&'static str>| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            req = req.header(header::ORIGIN, origin);
        }
        req.body(Body::from(
            serde_json::json!({ "username": "alex", "password": common::PASSWORD }).to_string(),
        ))
        .unwrap()
    };

    // Eigener Web-Client: Sitzung nur im HttpOnly-Cookie, kein Token im Body.
    let (status, headers, body) = app.send_full(login(Some("https://chat.example.org"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::SET_COOKIE).is_some());
    assert!(body.get("token").is_none(), "{body}");
    assert_eq!(body["user"]["username"], "alex");

    // Desktop-App (andere Origin) und Werkzeuge ohne Origin brauchen das Token für den Bearer-Betrieb.
    for origin in [Some("http://tauri.localhost"), None] {
        let (status, _, body) = app.send_full(login(origin)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()), "{origin:?}: {body}");
    }
}
