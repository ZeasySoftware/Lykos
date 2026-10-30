//! Bot-Konten: Anlegen/Token, Bot-Auth für REST und Gateway, zentrale Sperre für Konto-Endpunkte,
//! Hinzufügen zu Communities (Rechte, private Bots), Abschalten per Instanz-Einstellung, Konto-Löschen
//! des Besitzers und Datenexport.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

/// Anfrage mit `Authorization: Bot <token>`.
async fn bot_call(
    app: &TestApp,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(format!("/api{path}"))
        .header(header::AUTHORIZATION, format!("Bot {token}"));
    let req = match body {
        Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    app.send(req).await
}

/// Legt einen Bot an und gibt (id, token) zurück.
async fn create_bot(app: &TestApp, owner: &str, username: &str, public: bool) -> (String, String) {
    let (status, body) = app
        .call(
            "POST",
            "/bots",
            Some(owner),
            Some(json!({ "username": username, "description": "Hilft beim Testen", "is_public": public })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (s(&body["id"]).to_owned(), s(&body["token"]).to_owned())
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn create_and_regenerate_token(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (id, token) = create_bot(&app, &alex, "helfer", false).await;
    assert!(token.starts_with("lykb_"));

    let (_, list) = app.call("GET", "/bots", Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["username"], "helfer");
    assert!(list[0].get("token").is_none());

    // Bot authentifiziert sich und sieht sich selbst als Bot.
    let (status, me) = bot_call(&app, "GET", "/auth/me", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["id"], id.as_str());
    assert_eq!(me["bot"], true);
    // Als Bearer-Session gilt das Token nicht.
    let (status, _) = app.call("GET", "/auth/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Anmelden mit Passwort ist unmöglich (kein Passwort, keine Session für Bots).
    let (status, _) =
        app.call("POST", "/auth/login", None, Some(json!({ "username": "helfer", "password": "" }))).await;
    assert!(status.is_client_error());

    // Neues Token: altes ungültig.
    let (status, fresh) = app.call("POST", &format!("/bots/{id}/token"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = bot_call(&app, "GET", "/auth/me", &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = bot_call(&app, "GET", "/auth/me", s(&fresh["token"]), None).await;
    assert_eq!(status, StatusCode::OK);

    // Fremde sehen/ändern den privaten Bot nicht; Name belegt den Namensraum.
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let (status, _) = app.call("POST", &format!("/bots/{id}/token"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.call("GET", &format!("/bots/{id}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.call("POST", "/bots", Some(&bob), Some(json!({ "username": "HELFER" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Obergrenze 10 pro Nutzer.
    for i in 1..10 {
        create_bot(&app, &alex, &format!("bot{i}"), false).await;
    }
    let (status, body) = app.call("POST", "/bots", Some(&alex), Some(json!({ "username": "elf" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "too_many_bots");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn bots_cannot_use_account_endpoints(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (_, token) = create_bot(&app, &alex, "helfer", false).await;
    let community = app.community(&alex, "Rudel").await;
    let bob_id = app.user_id(&app.invited_user(&alex, &community, "bob").await).await;

    for (method, path, body) in [
        ("PATCH", "/auth/me", Some(json!({ "display_name": "x" }))),
        ("DELETE", "/auth/me", Some(json!({}))),
        ("GET", "/auth/me/export", None),
        ("GET", "/auth/sessions", None),
        ("POST", "/auth/password", Some(json!({ "current_password": "x", "new_password": "y" }))),
        ("POST", "/auth/mfa/totp/setup", Some(json!({}))),
        ("POST", "/relationships", Some(json!({ "username": "bob" }))),
        ("PUT", &format!("/relationships/{bob_id}"), Some(json!({ "type": "friend" }))),
        ("POST", "/dms", Some(json!({ "recipient_ids": [bob_id] }))),
        ("POST", "/bots", Some(json!({ "username": "kind" }))),
        ("POST", "/communities", Some(json!({ "name": "Bot-Land" }))),
        ("GET", "/admin/users", None),
        ("POST", "/invites/abcdefgh/accept", None),
    ] {
        let (status, res) = bot_call(&app, method, path, &token, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {res}");
        assert_eq!(res["error"], "bots_not_allowed", "{method} {path}");
    }

    // Freundschaftsanfragen an Bots gehen nicht.
    let (status, _) =
        app.call("POST", "/relationships", Some(&alex), Some(json!({ "username": "helfer" }))).await;
    assert!(status.is_client_error());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn add_to_community_and_chat(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let (bot_id, token) = create_bot(&app, &alex, "helfer", false).await;
    let carol = app.invited_user(&alex, &community, "carol").await;
    let (carol_bot, _) = create_bot(&app, &carol, "carolbot", false).await;
    let (public_bot, _) = create_bot(&app, &carol, "oeffentlich", true).await;
    let path = format!("/communities/{}/bots", community.id);

    // Ohne MANAGE_COMMUNITY: verboten.
    let (status, _) = app.call("POST", &path, Some(&bob), Some(json!({ "bot_id": bot_id }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Fremder privater Bot: nicht auffindbar; öffentlicher geht.
    let (status, _) = app.call("POST", &path, Some(&alex), Some(json!({ "bot_id": carol_bot }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.call("POST", &path, Some(&alex), Some(json!({ "bot_id": public_bot }))).await;
    assert_eq!(status, StatusCode::CREATED);
    // Ein normaler Nutzer ist kein Bot.
    let bob_id = app.user_id(&bob).await;
    let (status, _) = app.call("POST", &path, Some(&alex), Some(json!({ "bot_id": bob_id }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, member) = app.call("POST", &path, Some(&alex), Some(json!({ "bot_id": bot_id }))).await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    assert_eq!(member["user"]["bot"], true);
    let (status, _) = app.call("POST", &path, Some(&alex), Some(json!({ "bot_id": bot_id }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Mitgliederliste kennzeichnet Bots.
    let (_, members) =
        app.call("GET", &format!("/communities/{}/members", community.id), Some(&alex), None).await;
    let bot_member = members.as_array().unwrap().iter().find(|m| m["user"]["id"] == bot_id.as_str()).unwrap();
    assert_eq!(bot_member["user"]["bot"], true);
    let human = members.as_array().unwrap().iter().find(|m| m["user"]["id"] == bob_id.as_str()).unwrap();
    assert!(human["user"].get("bot").is_none());

    // Bot schreibt über REST – mit Embeds, Erwähnungen nach normalen Regeln.
    let (status, message) = bot_call(
        &app,
        "POST",
        &format!("/channels/{}/messages", community.text_channel),
        &token,
        Some(json!({ "content": "Hallo @bob @everyone", "embeds": [{ "title": "Status" }] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{message}");
    assert_eq!(message["author"]["bot"], true);
    assert_eq!(message["embeds"][0]["title"], "Status");
    assert_eq!(message["mention_everyone"], false);
    assert_eq!(message["mentions"], json!([bob_id]));
    // Menschen können keine Embeds senden.
    let (status, _) = app
        .call(
            "POST",
            &format!("/channels/{}/messages", community.text_channel),
            Some(&bob),
            Some(json!({ "content": "x", "embeds": [{ "title": "y" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Besitz kann nicht an Bots gehen.
    let (status, _) = app
        .call(
            "POST",
            &format!("/communities/{}/transfer", community.id),
            Some(&alex),
            Some(json!({ "user_id": bot_id })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Kicken wie bei Menschen.
    let (status, _) = app
        .call("DELETE", &format!("/communities/{}/members/{bot_id}", community.id), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = bot_call(
        &app,
        "POST",
        &format!("/channels/{}/messages", community.text_channel),
        &token,
        Some(json!({ "content": "noch da?" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn instance_admin_can_disable_bots(db: PgPool) {
    let app = TestApp::new(db);
    // Erster Nutzer ist Instanz-Admin.
    let alex = app.user("alex", None).await;
    let (_, token) = create_bot(&app, &alex, "helfer", false).await;
    let (_, info) = app.call("GET", "/instance", None, None).await;
    assert_eq!(info["allow_bots"], true);

    let (status, _) =
        app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "allow_bots": false }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app.call("POST", "/bots", Some(&alex), Some(json!({ "username": "neu" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "bots_disabled");
    let (status, _) = bot_call(&app, "GET", "/auth/me", &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) =
        app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "allow_bots": null }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = bot_call(&app, "GET", "/auth/me", &token, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn deleting_owner_deletes_bots_and_export_lists_them(db: PgPool) {
    let app = TestApp::new(db);
    let admin = app.user("admin", None).await;
    let community = app.community(&admin, "Rudel").await;
    let alex = app.invited_user(&admin, &community, "alex").await;
    let (bot_id, token) = create_bot(&app, &alex, "helfer", false).await;

    // Export enthält eigene Bots ohne Token.
    let req = Request::builder()
        .uri("/api/auth/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {alex}"))
        .body(Body::empty())
        .unwrap();
    let (status, export) = app.send(req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(export["bots"][0]["username"], "helfer");
    assert!(!export.to_string().contains("lykb_"));

    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&alex), Some(json!({ "password": common::PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _) = bot_call(&app, "GET", "/auth/me", &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1::uuid)")
        .bind(&bot_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!exists);
}

// --- Gateway ----------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn next(ws: &mut Ws) -> Option<Value> {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    return Some(serde_json::from_str::<Value>(&t).unwrap());
                }
                Some(Ok(tungstenite::Message::Close(_))) | None => return None,
                Some(Ok(_)) => continue,
                Some(Err(_)) => return None,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut).await.expect("kein Event binnen 5 s")
}

async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let event = next(ws).await.expect("Verbindung beendet");
        if event["t"] == kind {
            return event["d"].clone();
        }
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_identify_with_bot_token(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let (bot_id, token) = create_bot(&app, &alex, "helfer", false).await;
    let (status, _) = app
        .call(
            "POST",
            &format!("/communities/{}/bots", community.id),
            Some(&alex),
            Some(json!({ "bot_id": bot_id })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let addr = serve(&app).await;

    // Falsches Token → Verbindung wird geschlossen.
    let (mut bad, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    bad.send(tungstenite::Message::Text(
        json!({ "op": "identify", "token": "Bot lykb_falsch" }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next(&mut bad).await, None);

    // IDENTIFY mit „Bot <token>“.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    ws.send(tungstenite::Message::Text(
        json!({ "op": "identify", "token": format!("Bot {token}") }).to_string().into(),
    ))
    .await
    .unwrap();
    let ready = next_of(&mut ws, "READY").await;
    assert_eq!(ready["user"]["id"], bot_id.as_str());
    assert_eq!(ready["user"]["bot"], true);
    assert_eq!(ready["community_ids"], json!([community.id]));

    // Events kommen an: Nachricht eines Menschen.
    let (status, _) = app
        .call(
            "POST",
            &format!("/channels/{}/messages", community.text_channel),
            Some(&alex),
            Some(json!({ "content": "!ping" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let created = next_of(&mut ws, "MESSAGE_CREATE").await;
    assert_eq!(created["content"], "!ping");

    // Token neu erzeugen trennt die Verbindung.
    let (status, _) = app.call("POST", &format!("/bots/{bot_id}/token"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    loop {
        match next(&mut ws).await {
            None => break,
            Some(_) => continue,
        }
    }
}
