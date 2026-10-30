//! Integrationstests für Paket A: Rate-Limiting und Admin-Endpunkte.

mod common;

use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode, header},
};
use common::{PASSWORD, TestApp, s};
use http_body_util::BodyExt;
use lykos_server::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

/// Wie es der Client übergibt: optional mit `X-Forwarded-For` und/oder Socket-Adresse.
struct Origin<'a> {
    forwarded_for: Option<&'a str>,
    socket: Option<&'a str>,
}

const NO_ORIGIN: Origin<'static> = Origin { forwarded_for: None, socket: None };

/// POST mit JSON an `/api{path}`; liefert Status, Header und Body.
async fn post(app: &TestApp, path: &str, origin: &Origin<'_>, body: Value) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api{path}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(xff) = origin.forwarded_for {
        req = req.header("x-forwarded-for", xff);
    }
    let mut req = req.body(Body::from(body.to_string())).unwrap();
    if let Some(addr) = origin.socket {
        req.extensions_mut().insert(ConnectInfo(addr.parse::<SocketAddr>().unwrap()));
    }
    let res = app.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, headers, json)
}

async fn login(
    app: &TestApp,
    username: &str,
    password: &str,
    origin: &Origin<'_>,
) -> (StatusCode, HeaderMap, Value) {
    post(app, "/auth/login", origin, json!({ "username": username, "password": password })).await
}

fn assert_rate_limited((status, headers, body): &(StatusCode, HeaderMap, Value)) {
    assert_eq!(*status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
    let retry: u64 = headers[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!(retry > 0);
}

fn trusting_proxy() -> Config {
    Config { trust_proxy: true, ..Config::for_tests() }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn login_is_limited_per_username(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    app.invited_user(&alex, &community, "bob").await;

    for _ in 0..10 {
        let (status, _, body) = login(&app, "alex", "falsches-passwort", &NO_ORIGIN).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_credentials");
    }
    // Gesperrt – auch mit richtigem Passwort und anderer Schreibweise.
    assert_rate_limited(&login(&app, "ALEX", PASSWORD, &NO_ORIGIN).await);
    // Andere Konten sind nicht betroffen.
    assert_eq!(login(&app, "bob", PASSWORD, &NO_ORIGIN).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn username_lock_does_not_lock_out_other_ips(db: PgPool) {
    let app = TestApp::new(db);
    app.user("alex", None).await;
    let attacker = Origin { forwarded_for: None, socket: Some("198.51.100.66:4000") };
    for _ in 0..10 {
        assert_eq!(login(&app, "alex", "falsches-passwort", &attacker).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_rate_limited(&login(&app, "alex", PASSWORD, &attacker).await);
    // Der Kontoinhaber von einer anderen IP ist nicht ausgesperrt.
    let owner = Origin { forwarded_for: None, socket: Some("203.0.113.5:4000") };
    assert_eq!(login(&app, "alex", PASSWORD, &owner).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn successful_login_resets_username_counter(db: PgPool) {
    let app = TestApp::new(db);
    app.user("alex", None).await;
    for _ in 0..9 {
        login(&app, "alex", "falsches-passwort", &NO_ORIGIN).await;
    }
    assert_eq!(login(&app, "alex", PASSWORD, &NO_ORIGIN).await.0, StatusCode::OK);
    for _ in 0..9 {
        assert_eq!(login(&app, "alex", "falsches-passwort", &NO_ORIGIN).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login(&app, "alex", PASSWORD, &NO_ORIGIN).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn login_is_limited_per_forwarded_ip(db: PgPool) {
    let app = TestApp::with_config(db, trusting_proxy());
    app.user("alex", None).await;

    let attacker = Origin { forwarded_for: Some("198.51.100.1"), socket: Some("10.0.0.2:4000") };
    for i in 0..30 {
        let (status, _, _) = login(&app, &format!("gibtsnicht{i}"), "egal-egal-egal", &attacker).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_rate_limited(&login(&app, "alex", PASSWORD, &attacker).await);
    // Ein vom Client vorangestellter, gefälschter Eintrag hilft nicht: zählt der vom Proxy angehängte.
    let spoofed = Origin { forwarded_for: Some("203.0.113.9, 198.51.100.1"), socket: Some("10.0.0.2:4000") };
    assert_rate_limited(&login(&app, "alex", PASSWORD, &spoofed).await);
    // Andere Client-IP hinter demselben Proxy: frei.
    let other = Origin { forwarded_for: Some("198.51.100.2"), socket: Some("10.0.0.2:4000") };
    assert_eq!(login(&app, "alex", PASSWORD, &other).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn registration_is_limited_per_socket_ip(db: PgPool) {
    // Ohne LYKOS_TRUST_PROXY zählt die Socket-Adresse; X-Forwarded-For wird ignoriert.
    let app = TestApp::new(db);
    let client = |xff| Origin { forwarded_for: Some(xff), socket: Some("192.0.2.10:5555") };
    let register = |name: String| json!({ "username": name, "password": PASSWORD });

    let (status, _, _) = post(&app, "/auth/register", &client("1.1.1.1"), register("alex".into())).await;
    assert_eq!(status, StatusCode::CREATED);
    // Weitere Versuche ohne Einladung scheitern, zählen aber mit.
    for i in 0..4 {
        let xff = if i % 2 == 0 { "1.1.1.2" } else { "1.1.1.3" };
        let (status, _, _) = post(&app, "/auth/register", &client(xff), register(format!("user{i}"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert_rate_limited(&post(&app, "/auth/register", &client("1.1.1.4"), register("zuviel".into())).await);

    // Formal ungültige Anfragen zählen nicht, sie scheitern vorher.
    let (status, _, body) =
        post(&app, "/auth/register", &client("1.1.1.4"), json!({ "username": "x", "password": PASSWORD }))
            .await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_username")));

    // Andere Adresse: nicht gesperrt (nur keine Einladung).
    let other = Origin { forwarded_for: None, socket: Some("192.0.2.11:5555") };
    assert_eq!(post(&app, "/auth/register", &other, register("carol".into())).await.0, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn channel_overrides_can_be_listed(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let channel = &community.text_channel;

    let (status, list) = app.call("GET", &format!("/channels/{channel}/overrides"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list, json!([]));

    // Rolle „Team“ anlegen, Standardrolle verweigern, Team erlauben.
    let (_, team) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(&alex),
            Some(json!({ "name": "Team" })),
        )
        .await;
    let team_id = s(&team["id"]).to_owned();
    for (role, allow, deny) in [(&community.default_role, 0, 2), (&team_id, 2, 0)] {
        let (status, _) = app
            .call(
                "PUT",
                &format!("/channels/{channel}/overrides/{role}"),
                Some(&alex),
                Some(json!({ "allow": allow, "deny": deny })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    let (_, list) = app.call("GET", &format!("/channels/{channel}/overrides"), Some(&alex), None).await;
    assert_eq!(
        list,
        json!([
            { "role_id": community.default_role, "allow": 0, "deny": 2 },
            { "role_id": team_id, "allow": 2, "deny": 0 },
        ])
    );

    // Ohne MANAGE_ROLES: verboten.
    let (status, _) = app.call("GET", &format!("/channels/{channel}/overrides"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Override entfernen → Liste schrumpft; landet im Audit-Log.
    let (status, _) =
        app.call("DELETE", &format!("/channels/{channel}/overrides/{team_id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = app.call("GET", &format!("/channels/{channel}/overrides"), Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| s(&e["action"])).collect();
    assert!(actions.contains(&"channel.override_set"));
    assert!(actions.contains(&"channel.override_delete"));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn overrides_of_hidden_channel_are_not_found(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let channel = &community.text_channel;

    // Bob bekommt eine Rolle mit MANAGE_ROLES, der Kanal wird für alle unsichtbar.
    let (_, mods) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(&alex),
            Some(json!({ "name": "Mods", "permissions": 1 << 9 })),
        )
        .await;
    let bob_id = app.user_id(&bob).await;
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{}/members/{bob_id}/roles/{}", community.id, s(&mods["id"])),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.call("GET", &format!("/channels/{channel}/overrides"), Some(&bob), None).await.0,
        StatusCode::OK
    );

    app.call(
        "PUT",
        &format!("/channels/{channel}/overrides/{}", community.default_role),
        Some(&alex),
        Some(json!({ "allow": 0, "deny": 1 })),
    )
    .await;
    let (status, _) = app.call("GET", &format!("/channels/{channel}/overrides"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
