//! Integrationstests für Paket H: Instanz-Verwaltung (Einladungen, Nutzer, Communities, Einstellungen,
//! Übersicht, Audit-Log, Passwort-Reset-Links).

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::StatusCode;
use common::{PASSWORD, TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// alex (erster Nutzer = Instanz-Admin, Besitzer von „Rudel“) und bob (Mitglied, kein Admin).
async fn setup(app: &TestApp) -> (String, String, TestCommunity) {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    (alex, bob, community)
}

async fn login(app: &TestApp, username: &str, password: &str) -> (StatusCode, Value) {
    app.call("POST", "/auth/login", None, Some(json!({ "username": username, "password": password }))).await
}

async fn create_invite(app: &TestApp, admin: &str, body: Value) -> Value {
    let (status, invite) = app.call("POST", "/admin/invites", Some(admin), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{invite}");
    invite
}

async fn serve(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn connect(addr: SocketAddr, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    let identify = json!({ "op": "identify", "token": token }).to_string();
    ws.send(tungstenite::Message::Text(identify.into())).await.unwrap();
    let ready = next_text(&mut ws).await.expect("READY");
    assert_eq!(ready["t"], "READY");
    ws
}

/// Nächstes Text-Event (max. 5 s); `None` bei Close/Ende.
async fn next_text(ws: &mut Ws) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => return Some(serde_json::from_str(&t).unwrap()),
                Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => return None,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .expect("kein Event binnen 5 s")
}

/// Wartet auf das Schließen der Verbindung und liefert den Close-Code.
async fn closed_with(ws: &mut Ws) -> Option<u16> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Close(frame))) => return frame.map(|f| u16::from(f.code)),
                Some(Ok(_)) => continue,
                _ => return None,
            }
        }
    })
    .await
    .expect("Verbindung nicht geschlossen")
}

/// Token aus einem Reset-Link `…/reset-password#token=…`.
fn token_of(url: &str) -> &str {
    url.split("token=").nth(1).unwrap()
}

// --- Zugriff ------------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn admin_routes_require_instance_admin(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;

    let routes: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", "/admin/overview".into(), None),
        ("GET", "/admin/invites".into(), None),
        ("POST", "/admin/invites".into(), Some(json!({}))),
        ("DELETE", "/admin/invites/abc".into(), None),
        ("GET", "/admin/users".into(), None),
        ("GET", format!("/admin/users/{bob_id}"), None),
        ("POST", format!("/admin/users/{bob_id}/disable"), None),
        ("PUT", format!("/admin/users/{bob_id}/admin"), None),
        ("POST", format!("/admin/users/{bob_id}/password-reset"), None),
        ("DELETE", format!("/admin/users/{bob_id}"), Some(json!({}))),
        ("GET", "/admin/communities".into(), None),
        ("DELETE", format!("/admin/communities/{}", community.id), None),
        ("GET", "/admin/settings".into(), None),
        ("PATCH", "/admin/settings".into(), Some(json!({ "registration_mode": "open" }))),
        ("GET", "/admin/audit-log".into(), None),
    ];
    for (method, path, body) in &routes {
        let (status, res) = app.call(method, path, Some(&bob), body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {res}");
        assert_eq!(res["error"], "forbidden");
        let (status, _) = app.call(method, path, None, body.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path} ohne Anmeldung");
    }
    // Nichts davon hat gewirkt.
    let (status, _) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, overview) = app.call("GET", "/admin/overview", Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["users"]["total"], 2);
    assert_eq!(overview["users"]["admins"], 1);
    assert_eq!(overview["communities"], 1);
    assert!(overview["database_bytes"].as_i64().unwrap() > 0);
    assert!(overview["version"].is_string());
}

// --- Instanz-Einladungen -------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn instance_invite_registers_and_joins_community(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;

    let invite = create_invite(
        &app,
        &alex,
        json!({ "max_uses": 1, "expires_in_hours": 24, "note": "Für Carol", "community_id": community.id }),
    )
    .await;
    let code = s(&invite["code"]).to_owned();
    assert_eq!(code.len(), 12);
    assert_eq!(invite["url"], format!("http://localhost/invite/{code}"));
    assert_eq!(invite["status"], "active");
    assert_eq!(invite["note"], "Für Carol");
    assert_eq!(invite["community"]["name"], "Rudel");

    // Öffentliche Vorschau über dieselbe Route wie Community-Einladungen.
    let (status, preview) = app.call("GET", &format!("/invites/{code}"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["kind"], "instance");
    assert_eq!(preview["instance_name"], "Test");
    assert_eq!(preview["community"]["name"], "Rudel");

    // Bestehende Nutzer können eine Instanz-Einladung nicht „annehmen“.
    let (status, res) = app.call("POST", &format!("/invites/{code}/accept"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{res}");
    assert_eq!(res["error"], "instance_invite");

    // Registrierung bei invite_only mit Instanz-Einladung → automatisch Mitglied.
    let carol = app.user("carol", Some(&code)).await;
    let (_, list) = app.call("GET", "/communities", Some(&carol), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], community.id.as_str());

    // Aufgebraucht: zweite Registrierung scheitert, ohne ein Konto anzulegen.
    let (status, res) = app.register("dave", Some(&code)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{res}");
    assert_eq!(res["error"], "invalid_invite");
    let (status, _) = login(&app, "dave", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app.call("GET", &format!("/invites/{code}"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Liste: Nutzungen, Ersteller und wer sich registriert hat.
    let (_, invites) = app.call("GET", "/admin/invites", Some(&alex), None).await;
    let listed = &invites[0];
    assert_eq!(listed["code"], code.as_str());
    assert_eq!(listed["uses"], 1);
    assert_eq!(listed["status"], "used_up");
    assert_eq!(listed["created_by"]["username"], "alex");
    assert_eq!(listed["used_by"][0]["user"]["username"], "carol");

    // Nutzerdetail zeigt die Einladung.
    let carol_id = app.user_id(&carol).await;
    let (_, detail) = app.call("GET", &format!("/admin/users/{carol_id}"), Some(&alex), None).await;
    assert_eq!(detail["registered_with"]["code"], code.as_str());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn instance_invite_without_community_expired_and_revoked(db: PgPool) {
    let app = TestApp::new(db.clone());
    let (alex, _bob, _community) = setup(&app).await;

    // Ohne Community: Registrierung klappt, keine Mitgliedschaft.
    let open = create_invite(&app, &alex, json!({})).await;
    let (_, preview) = app.call("GET", &format!("/invites/{}", s(&open["code"])), None, None).await;
    assert_eq!(preview["kind"], "instance");
    assert!(preview["community"].is_null());
    let erin = app.user("erin", Some(s(&open["code"]))).await;
    let (_, list) = app.call("GET", "/communities", Some(&erin), None).await;
    assert!(list.as_array().unwrap().is_empty());

    // Abgelaufen.
    let expired = create_invite(&app, &alex, json!({ "expires_in_hours": 1 })).await;
    sqlx::query("UPDATE instance_invites SET expires_at = now() - interval '1 minute' WHERE code = $1")
        .bind(s(&expired["code"]))
        .execute(&db)
        .await
        .unwrap();
    let (status, res) = app.register("frank", Some(s(&expired["code"]))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(res["error"], "invalid_invite");

    // Widerrufen.
    let revoked = create_invite(&app, &alex, json!({})).await;
    let (status, _) =
        app.call("DELETE", &format!("/admin/invites/{}", s(&revoked["code"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.register("frank", Some(s(&revoked["code"]))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, invites) = app.call("GET", "/admin/invites", Some(&alex), None).await;
    let status_of = |code: &Value| {
        invites.as_array().unwrap().iter().find(|i| i["code"] == *code).unwrap()["status"].clone()
    };
    assert_eq!(status_of(&expired["code"]), "expired");
    assert_eq!(status_of(&revoked["code"]), "revoked");
    assert_eq!(status_of(&open["code"]), "active");

    // Ungültige Angaben.
    for body in
        [json!({ "max_uses": 0 }), json!({ "expires_in_hours": 0 }), json!({ "note": "x".repeat(201) })]
    {
        let (status, _) = app.call("POST", "/admin/invites", Some(&alex), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = app
        .call("POST", "/admin/invites", Some(&alex), Some(json!({ "community_id": uuid::Uuid::now_v7() })))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.call("DELETE", "/admin/invites/gibtsnicht", Some(&alex), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn registration_modes(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, _community) = setup(&app).await;
    let invite = create_invite(&app, &alex, json!({})).await;
    let code = s(&invite["code"]).to_owned();

    // invite_only (Standard): ohne Einladung nicht.
    let (status, res) = app.register("gina", None).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::FORBIDDEN, json!("invite_required")));

    // closed: auch mit gültiger Instanz-Einladung nicht.
    let (status, settings) = app
        .call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "registration_mode": "closed" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["registration_mode"], "closed");
    let (_, info) = app.call("GET", "/instance", None, None).await;
    assert_eq!(info["registration_mode"], "closed");
    let (status, res) = app.register("gina", Some(&code)).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::FORBIDDEN, json!("registration_closed")));

    // open: ohne Einladung möglich.
    app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "registration_mode": "open" }))).await;
    app.user("gina", None).await;

    let (status, _) =
        app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "registration_mode": "egal" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// --- Nutzer ---------------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn disabled_user_cannot_log_in_and_is_disconnected(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    let alex_id = app.user_id(&alex).await;
    let addr = serve(&app).await;
    let mut ws = connect(addr, &bob).await;

    let (status, _) = app.call("POST", &format!("/admin/users/{bob_id}/disable"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Gateway getrennt (Session widerrufen), Token ungültig, Anmeldung verweigert.
    assert_eq!(closed_with(&mut ws).await, Some(4001));
    let (status, _) = app.call("GET", "/auth/me", Some(&bob), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, res) = login(&app, "bob", PASSWORD).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::FORBIDDEN, json!("account_disabled")));
    // Falsches Passwort verrät die Sperre nicht.
    let (status, _) = login(&app, "bob", "falsches-passwort").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (_, list) = app.call("GET", "/admin/users?filter=disabled", Some(&alex), None).await;
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["username"], "bob");
    assert!(list["items"][0]["disabled_at"].is_string());

    // Sich selbst sperren geht nicht.
    let (status, res) = app.call("POST", &format!("/admin/users/{alex_id}/disable"), Some(&alex), None).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::CONFLICT, json!("cannot_target_self")));

    // Entsperren → Anmeldung wieder möglich.
    let (status, _) = app.call("POST", &format!("/admin/users/{bob_id}/enable"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = login(&app, "bob", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn revoke_sessions_disconnects(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    let addr = serve(&app).await;
    let mut ws = connect(addr, &bob).await;

    let (status, _) = app.call("DELETE", &format!("/admin/users/{bob_id}/sessions"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(closed_with(&mut ws).await, Some(4001));
    let (status, _) = app.call("GET", "/auth/me", Some(&bob), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Nicht gesperrt: neu anmelden geht.
    let (status, _) = login(&app, "bob", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn last_admin_is_protected(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _community) = setup(&app).await;
    let (alex_id, bob_id) = (app.user_id(&alex).await, app.user_id(&bob).await);

    let (status, res) = app.call("DELETE", &format!("/admin/users/{alex_id}/admin"), Some(&alex), None).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::CONFLICT, json!("last_instance_admin")));

    // bob wird Admin, darf jetzt die Verwaltung nutzen.
    let (status, _) = app.call("PUT", &format!("/admin/users/{bob_id}/admin"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("GET", "/admin/overview", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, me) = app.call("GET", "/auth/me", Some(&bob), None).await;
    assert_eq!(me["is_instance_admin"], true);

    // alex gibt sein Recht ab; bob ist jetzt der letzte Admin und kann es nicht abgeben.
    let (status, _) = app.call("DELETE", &format!("/admin/users/{alex_id}/admin"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("GET", "/admin/overview", Some(&alex), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, res) = app.call("DELETE", &format!("/admin/users/{bob_id}/admin"), Some(&bob), None).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::CONFLICT, json!("last_instance_admin")));

    // Audit-Log hält beides fest.
    let (_, log) = app.call("GET", "/admin/audit-log", Some(&bob), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();
    assert_eq!(actions, ["user.admin_revoke", "user.admin_grant"]);
    assert_eq!(log[0]["data"]["username"], "alex");
    assert_eq!(log[0]["actor"]["username"], "alex");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn admin_password_reset_link(db: PgPool) {
    let app = TestApp::new(db.clone());
    let (alex, bob, _community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;

    let (status, link) =
        app.call("POST", &format!("/admin/users/{bob_id}/password-reset"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::CREATED, "{link}");
    let url = s(&link["url"]).to_owned();
    assert!(url.starts_with("http://localhost/reset-password#token="), "{url}");
    let token = token_of(&url).to_owned();

    // Nur der Hash liegt in der Datenbank.
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM password_reset_tokens WHERE kind = 'admin_reset'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_ne!(stored, token.as_bytes());

    let (status, info) =
        app.call("POST", "/auth/password-reset/verify", None, Some(json!({ "token": token }))).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["username"], "bob");

    // Zu kurzes Passwort → nichts verbraucht.
    let (status, _) = app
        .call(
            "POST",
            "/auth/password-reset/complete",
            None,
            Some(json!({ "token": token, "new_password": "kurz" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let new_password = "ganz-neues-passwort-123";
    let (status, _) = app
        .call(
            "POST",
            "/auth/password-reset/complete",
            None,
            Some(json!({ "token": token, "new_password": new_password })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Alte Sessions beendet, altes Passwort ungültig, neues gilt.
    let (status, _) = app.call("GET", "/auth/me", Some(&bob), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "bob", PASSWORD).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "bob", new_password).await.0, StatusCode::OK);

    // Einmalig.
    let (status, res) = app
        .call(
            "POST",
            "/auth/password-reset/complete",
            None,
            Some(json!({ "token": token, "new_password": "noch-ein-passwort" })),
        )
        .await;
    assert_eq!((status, res["error"].clone()), (StatusCode::NOT_FOUND, json!("invalid_reset_token")));

    // Ein neuer Link macht den vorherigen ungültig; abgelaufene gelten nicht.
    let (_, first) =
        app.call("POST", &format!("/admin/users/{bob_id}/password-reset"), Some(&alex), None).await;
    let (_, second) =
        app.call("POST", &format!("/admin/users/{bob_id}/password-reset"), Some(&alex), None).await;
    let verify = |t: String| {
        let app = &app;
        async move { app.call("POST", "/auth/password-reset/verify", None, Some(json!({ "token": t }))).await.0 }
    };
    assert_eq!(verify(token_of(s(&first["url"])).to_owned()).await, StatusCode::NOT_FOUND);
    assert_eq!(verify(token_of(s(&second["url"])).to_owned()).await, StatusCode::OK);
    sqlx::query("UPDATE password_reset_tokens SET expires_at = now() - interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(verify(token_of(s(&second["url"])).to_owned()).await, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn delete_user_handles_owned_communities(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, rudel) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    let alex_id = app.user_id(&alex).await;
    // bob besitzt zwei Communities: eine mit weiterem Mitglied (carol), eine allein.
    let shared = app.community(&bob, "Geteilt").await;
    let carol = app.invited_user(&bob, &shared, "carol").await;
    let solo = app.community(&bob, "Allein").await;
    let carol_id = app.user_id(&carol).await;

    // Ohne Angabe → klare Meldung.
    let (status, res) =
        app.call("DELETE", &format!("/admin/users/{bob_id}"), Some(&alex), Some(json!({}))).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::CONFLICT, json!("owns_communities")));
    // Sich selbst löschen geht hier nicht.
    let (status, _) =
        app.call("DELETE", &format!("/admin/users/{alex_id}"), Some(&alex), Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = app
        .call(
            "DELETE",
            &format!("/admin/users/{bob_id}"),
            Some(&alex),
            Some(json!({ "owned_communities": "transfer", "delete_messages": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // „Geteilt“ gehört jetzt carol, „Allein“ ist gelöscht, „Rudel“ unverändert.
    let (_, detail) = app.call("GET", &format!("/communities/{}", shared.id), Some(&carol), None).await;
    assert_eq!(detail["community"]["owner_id"], carol_id.as_str());
    let (_, communities) = app.call("GET", "/admin/communities", Some(&alex), None).await;
    let names: Vec<&str> =
        communities["items"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"Allein"), "{names:?}");
    assert!(names.contains(&"Rudel") && names.contains(&"Geteilt"));
    let _ = (rudel, solo);

    assert_eq!(login(&app, "bob", PASSWORD).await.0, StatusCode::UNAUTHORIZED);
    let (status, _) = app.call("GET", &format!("/admin/users/{bob_id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, log) = app.call("GET", "/admin/audit-log", Some(&alex), None).await;
    assert_eq!(log[0]["action"], "user.delete");
    assert_eq!(log[0]["data"]["username"], "bob");
    assert_eq!(log[0]["data"]["communities"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn user_list_search_and_paging(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, community) = setup(&app).await;
    for name in ["wolf_1", "wolf_2", "wolfgang"] {
        app.invited_user(&alex, &community, name).await;
    }
    let (_, page) = app.call("GET", "/admin/users?q=wolf&per_page=2", Some(&alex), None).await;
    assert_eq!(page["total"], 3);
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    let (_, page2) = app.call("GET", "/admin/users?q=wolf&per_page=2&page=1", Some(&alex), None).await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    // `_` ist kein Platzhalter.
    let (_, exact) = app.call("GET", "/admin/users?q=wolf_", Some(&alex), None).await;
    assert_eq!(exact["total"], 2);

    let (_, admins) = app.call("GET", "/admin/users?filter=admins", Some(&alex), None).await;
    assert_eq!(admins["total"], 1);
    let first = &admins["items"][0];
    assert_eq!(first["username"], "alex");
    assert_eq!(first["community_count"], 1);
    assert_eq!(first["is_instance_admin"], true);
    // Nur der Tag der letzten Aktivität.
    assert_eq!(first["last_active_on"].as_str().unwrap().len(), 10);
}

// --- Communities ------------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn admin_manages_foreign_communities(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, rudel) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    // carol gründet eine Community, in der alex nicht Mitglied ist.
    let invite = create_invite(&app, &alex, json!({ "community_id": rudel.id })).await;
    let carol = app.user("carol", Some(s(&invite["code"]))).await;
    let bau = app.community(&carol, "Bau").await;
    let (_, inv) =
        app.call("POST", &format!("/communities/{}/invites", bau.id), Some(&carol), Some(json!({}))).await;
    app.call("POST", &format!("/invites/{}/accept", s(&inv["code"])), Some(&bob), None).await;

    let (_, list) = app.call("GET", "/admin/communities?q=bau", Some(&alex), None).await;
    assert_eq!(list["total"], 1);
    let item = &list["items"][0];
    assert_eq!(item["owner"]["username"], "carol");
    assert_eq!(item["member_count"], 2);
    assert_eq!(item["storage_bytes"], 0);

    let (_, members) =
        app.call("GET", &format!("/admin/communities/{}/members", bau.id), Some(&alex), None).await;
    assert_eq!(members.as_array().unwrap().len(), 2);

    // Besitz übertragen ohne Mitglied zu sein; Ziel muss Mitglied sein.
    let alex_id = app.user_id(&alex).await;
    let (status, _) = app
        .call(
            "POST",
            &format!("/admin/communities/{}/transfer", bau.id),
            Some(&alex),
            Some(json!({ "user_id": alex_id })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, community) = app
        .call(
            "POST",
            &format!("/admin/communities/{}/transfer", bau.id),
            Some(&alex),
            Some(json!({ "user_id": bob_id })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{community}");
    assert_eq!(community["owner_id"], bob_id.as_str());

    // Löschen.
    let (status, _) = app.call("DELETE", &format!("/admin/communities/{}", bau.id), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("GET", &format!("/communities/{}", bau.id), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, log) = app.call("GET", "/admin/audit-log", Some(&alex), None).await;
    assert_eq!(log[0]["action"], "community.delete");
    assert_eq!(log[0]["data"]["name"], "Bau");
    assert_eq!(log[1]["action"], "community.transfer");
}

// --- Einstellungen ---------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn runtime_settings_and_instance_update(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _community) = setup(&app).await;
    let addr = serve(&app).await;
    let mut ws = connect(addr, &bob).await;

    let (status, settings) = app.call("GET", "/admin/settings", Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["instance_name"], "Test");
    assert_eq!(settings["max_upload_mb"], 100);
    assert_eq!(settings["user_quota_mb"], 2048);
    assert_eq!(settings["allow_community_creation"], true);
    assert_eq!(settings["defaults"]["max_upload_mb"], 100);

    let (status, settings) = app
        .call(
            "PATCH",
            "/admin/settings",
            Some(&alex),
            Some(json!({
                "instance_name": "Rudelhaus",
                "max_upload_mb": 5,
                "user_quota_mb": 0,
                "allow_community_creation": false,
                "announcement": "  Wartung heute um 22 Uhr  ",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    assert_eq!(settings["user_quota_mb"], Value::Null);
    assert_eq!(settings["announcement"], "Wartung heute um 22 Uhr");

    // Alle Verbindungen bekommen INSTANCE_UPDATE.
    let event = loop {
        let e = next_text(&mut ws).await.expect("Event");
        if e["t"] == "INSTANCE_UPDATE" {
            break e["d"].clone();
        }
    };
    assert_eq!(event["name"], "Rudelhaus");
    assert_eq!(event["announcement"], "Wartung heute um 22 Uhr");

    let (_, info) = app.call("GET", "/instance", None, None).await;
    assert_eq!(info["name"], "Rudelhaus");
    assert_eq!(info["max_upload_bytes"], 5 * 1024 * 1024);
    assert_eq!(info["allow_community_creation"], false);
    let (_, usage) = app.call("GET", "/auth/me/usage", Some(&bob), None).await;
    assert_eq!(usage["quota_bytes"], Value::Null);

    // Nur Admins dürfen Communities anlegen.
    let (status, res) = app.call("POST", "/communities", Some(&bob), Some(json!({ "name": "Neu" }))).await;
    assert_eq!((status, res["error"].clone()), (StatusCode::FORBIDDEN, json!("community_creation_disabled")));
    app.community(&alex, "Admin darf").await;

    // Zurücksetzen auf die Umgebung (`null`).
    let (_, settings) = app
        .call(
            "PATCH",
            "/admin/settings",
            Some(&alex),
            Some(json!({ "instance_name": null, "max_upload_mb": null, "user_quota_mb": null, "announcement": null })),
        )
        .await;
    assert_eq!(settings["instance_name"], "Test");
    assert_eq!(settings["max_upload_mb"], 100);
    assert_eq!(settings["user_quota_mb"], 2048);
    assert_eq!(settings["announcement"], Value::Null);

    for body in [
        json!({ "instance_name": "  " }),
        json!({ "max_upload_mb": 0 }),
        json!({ "announcement": "x".repeat(501) }),
    ] {
        let (status, _) = app.call("PATCH", "/admin/settings", Some(&alex), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    let (_, log) = app.call("GET", "/admin/audit-log", Some(&alex), None).await;
    assert_eq!(log[0]["action"], "settings.update");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn upload_limit_follows_runtime_setting(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, community) = setup(&app).await;
    app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "max_upload_mb": 1 }))).await;

    let boundary = "lykosgrenze";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"gross.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend(vec![0u8; 1024 * 1024 + 10]);
    body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{}/attachments", community.text_channel))
        .header(axum::http::header::AUTHORIZATION, format!("Bearer {alex}"))
        .header(axum::http::header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
        .body(axum::body::Body::from(body))
        .unwrap();
    let (status, res) = app.send(req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{res}");
    assert_eq!(res["error"], "file_too_large");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn disabling_kicks_from_voice(db: PgPool) {
    let app = TestApp::with_config(db.clone(), lykos_server::config::Config::for_tests_with_voice());
    let (alex, bob, community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    // Voice-Status wie nach dem LiveKit-Webhook `participant_joined`.
    sqlx::query(
        "INSERT INTO voice_states (user_id, channel_id, community_id) VALUES ($1::uuid, $2::uuid, $3::uuid)",
    )
    .bind(&bob_id)
    .bind(&community.voice_channel)
    .bind(&community.id)
    .execute(&db)
    .await
    .unwrap();
    let (_, overview) = app.call("GET", "/admin/overview", Some(&alex), None).await;
    assert_eq!(overview["voice"]["participants"], 1);
    assert_eq!(overview["voice"]["channels"][0]["channel_name"], "Sprachkanal");

    let (status, _) = app.call("POST", &format!("/admin/users/{bob_id}/disable"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_states").fetch_one(&db).await.unwrap();
    assert_eq!(left, 0);
}

// --- Zusammenspiel mit Paket J/K -------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn instance_invite_join_posts_system_message_and_preview_has_community(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, rudel) = setup(&app).await;
    let (status, res) = app
        .call(
            "PATCH",
            &format!("/communities/{}", rudel.id),
            Some(&alex),
            Some(json!({ "system_channel_id": rudel.text_channel, "description": "Unser Rudel" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let invite = create_invite(&app, &alex, json!({ "community_id": rudel.id })).await;
    let code = s(&invite["code"]).to_owned();

    let (_, preview) = app.call("GET", &format!("/invites/{code}"), None, None).await;
    assert_eq!(preview["kind"], "instance");
    assert_eq!(preview["community"]["description"], "Unser Rudel");

    let carol = app.user("carol", Some(&code)).await;
    let carol_id = app.user_id(&carol).await;
    let (_, msgs) =
        app.call("GET", &format!("/channels/{}/messages", rudel.text_channel), Some(&alex), None).await;
    let joins: Vec<&Value> = msgs.as_array().unwrap().iter().filter(|m| m["kind"] == "member_join").collect();
    assert!(joins.iter().any(|m| m["author"]["id"] == carol_id.as_str()), "{msgs}");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn feedback_admin_uses_instance_guard_and_audit(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _rudel) = setup(&app).await;
    let (status, report) = app
        .call(
            "POST",
            "/feedback",
            Some(&bob),
            Some(json!({ "kind": "bug", "title": "Absturz", "description": "Geht nicht" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{report}");
    let id = s(&report["id"]).to_owned();

    let (status, _) = app.call("GET", "/admin/feedback", Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, res) = app
        .call("PATCH", &format!("/admin/feedback/{id}"), Some(&alex), Some(json!({ "status": "in_arbeit" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (_, log) = app.call("GET", "/admin/audit-log", Some(&alex), None).await;
    assert_eq!(log[0]["action"], "feedback.status");
    assert_eq!(log[0]["target_id"], id.as_str());

    // Datenexport: eigene Meldungen und Laufzeit-Instanzname.
    app.call("PATCH", "/admin/settings", Some(&alex), Some(json!({ "instance_name": "Rudelhaus" }))).await;
    let (status, export) = app.call("GET", "/auth/me/export", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(export["instance"], "Rudelhaus");
    assert_eq!(export["feedback"][0]["id"], id.as_str());
}
