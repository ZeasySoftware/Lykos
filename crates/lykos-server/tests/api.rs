//! Integrationstests gegen eine echte Postgres-Datenbank (`DATABASE_URL`).
//! `sqlx::test` legt pro Test eine frische Datenbank an und führt die Migrationen aus.

mod common;

use axum::http::StatusCode;
use common::{TestApp, s};
use futures_util::StreamExt;
use lykos_server::retention;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn bootstrap_invite_and_messaging(db: PgPool) {
    let app = TestApp::new(db);

    let (_, instance) = app.call("GET", "/instance", None, None).await;
    assert_eq!(instance["bootstrap"], true);
    assert_eq!(instance["registration_mode"], "invite_only");
    // AGPL §13: Quellcode der laufenden Fassung ist ohne Anmeldung auffindbar.
    assert_eq!(instance["source_url"], lykos_server::config::DEFAULT_SOURCE_URL);

    // Erster Nutzer darf ohne Einladung und wird Admin.
    let (status, body) = app.register("alex", None).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["user"]["is_instance_admin"], true);
    let alex = s(&body["token"]).to_owned();

    // Danach nur noch mit Einladung.
    assert_eq!(app.register("bob", None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.register("bob", Some("gibtsnicht")).await.0, StatusCode::NOT_FOUND);

    // Community anlegen: Standardkanäle sind da.
    let (status, detail) =
        app.call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let community_id = s(&detail["community"]["id"]).to_owned();
    let channels = detail["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    let general = channels.iter().find(|c| c["kind"] == "text").unwrap();
    let general_id = s(&general["id"]).to_owned();
    assert_eq!(general["name"], "allgemein");

    // Einladung mit einer Nutzung.
    let (status, invite) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/invites"),
            Some(&alex),
            Some(json!({ "max_uses": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = s(&invite["code"]).to_owned();

    let (status, preview) = app.call("GET", &format!("/invites/{code}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["community"]["name"], "Rudel");

    // Bob registriert sich mit der Einladung und ist sofort Mitglied.
    let bob = app.user("bob", Some(&code)).await;
    let (_, communities) = app.call("GET", "/communities", Some(&bob), None).await;
    assert_eq!(communities.as_array().unwrap().len(), 1);

    // Einladung ist verbraucht.
    assert_eq!(app.register("carol", Some(&code)).await.0, StatusCode::NOT_FOUND);

    // Doppelter Benutzername (ohne Groß/Klein-Unterschied).
    let (_, invite2) =
        app.call("POST", &format!("/communities/{community_id}/invites"), Some(&alex), Some(json!({}))).await;
    let (status, body) = app.register("BOB", Some(s(&invite2["code"]))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "username_taken");

    // Nachrichten.
    let (status, msg) = app
        .call(
            "POST",
            &format!("/channels/{general_id}/messages"),
            Some(&bob),
            Some(json!({ "content": "Hallo!" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(msg["author"]["username"], "bob");
    let msg_id = s(&msg["id"]).to_owned();

    let (status, _) = app
        .call(
            "POST",
            &format!("/channels/{general_id}/messages"),
            Some(&bob),
            Some(json!({ "content": "   " })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, list) = app.call("GET", &format!("/channels/{general_id}/messages"), Some(&alex), None).await;
    // Neben Bobs Nachricht steht seine Beitrittsmeldung (der erste Textkanal ist Systemkanal).
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["content"], "Hallo!");
    assert_eq!(list[1]["kind"], "member_join");

    // Fremde Nachricht bearbeiten: nein. Als Besitzer löschen: ja (MANAGE_MESSAGES).
    let (status, _) =
        app.call("PATCH", &format!("/messages/{msg_id}"), Some(&alex), Some(json!({ "content": "x" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, msg) = app
        .call(
            "PATCH",
            &format!("/messages/{msg_id}"),
            Some(&bob),
            Some(json!({ "content": "Hallo zusammen!" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!msg["edited_at"].is_null());
    assert_eq!(
        app.call("DELETE", &format!("/messages/{msg_id}"), Some(&alex), None).await.0,
        StatusCode::NO_CONTENT
    );

    // Löschen durch Moderation landet im Audit-Log.
    let (status, log) =
        app.call("GET", &format!("/communities/{community_id}/audit-log"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(log.as_array().unwrap().iter().any(|e| e["action"] == "message.delete"));
    // Bob darf das Audit-Log nicht sehen.
    let (status, _) =
        app.call("GET", &format!("/communities/{community_id}/audit-log"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn permissions_and_private_channels(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (_, detail) = app.call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel" }))).await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let default_role = detail["roles"].as_array().unwrap().iter().find(|r| r["is_default"] == true).unwrap();
    let default_role_id = s(&default_role["id"]).to_owned();

    let (_, invite) =
        app.call("POST", &format!("/communities/{community_id}/invites"), Some(&alex), Some(json!({}))).await;
    let bob = app.user("bob", Some(s(&invite["code"]))).await;

    // Bob darf keine Kanäle anlegen.
    let (status, _) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/channels"),
            Some(&bob),
            Some(json!({ "name": "geheim", "kind": "text" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Privater Kanal: Standardrolle sieht ihn nicht.
    let (status, channel) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/channels"),
            Some(&alex),
            Some(json!({ "name": "Team Intern", "kind": "text" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(channel["name"], "team-intern");
    let private_id = s(&channel["id"]).to_owned();
    let (status, _) = app
        .call(
            "PUT",
            &format!("/channels/{private_id}/overrides/{default_role_id}"),
            Some(&alex),
            Some(json!({ "allow": 0, "deny": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, bob_detail) = app.call("GET", &format!("/communities/{community_id}"), Some(&bob), None).await;
    assert!(!bob_detail["channels"].as_array().unwrap().iter().any(|c| c["id"] == private_id.as_str()));
    let (status, _) = app.call("GET", &format!("/channels/{private_id}/messages"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Rolle „Team“ mit Sichtrecht im Kanal → Bob sieht ihn.
    let (status, role) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/roles"),
            Some(&alex),
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let team_id = s(&role["id"]).to_owned();
    app.call(
        "PUT",
        &format!("/channels/{private_id}/overrides/{team_id}"),
        Some(&alex),
        Some(json!({ "allow": 1, "deny": 0 })),
    )
    .await;
    let (_, me) = app.call("GET", "/auth/me", Some(&bob), None).await;
    let bob_id = s(&me["id"]).to_owned();
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{community_id}/members/{bob_id}/roles/{team_id}"),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.call("GET", &format!("/channels/{private_id}/messages"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);

    // Rechte-Eskalation: Bob mit MANAGE_ROLES kann sich nicht ADMINISTRATOR geben.
    let (_, mods) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/roles"),
            Some(&alex),
            Some(json!({ "name": "Mods", "permissions": 1 << 9 })),
        )
        .await;
    let mods_id = s(&mods["id"]).to_owned();
    app.call(
        "PUT",
        &format!("/communities/{community_id}/members/{bob_id}/roles/{mods_id}"),
        Some(&alex),
        None,
    )
    .await;
    let (status, _) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/roles"),
            Some(&bob),
            Some(json!({ "name": "Gott", "permissions": 1 << 20 })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Besitzer kann nicht gekickt werden und nicht austreten; Bob kann austreten.
    let (_, alex_me) = app.call("GET", "/auth/me", Some(&alex), None).await;
    let alex_id = s(&alex_me["id"]).to_owned();
    let (status, body) = app
        .call("DELETE", &format!("/communities/{community_id}/members/{alex_id}"), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "owner_cannot_leave");
    let (status, _) =
        app.call("DELETE", &format!("/communities/{community_id}/members/{bob_id}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("GET", &format!("/communities/{community_id}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn sessions_login_logout_revoke(db: PgPool) {
    let app = TestApp::new(db);
    let first = app.user("alex", None).await;

    let (status, body) = app
        .call(
            "POST",
            "/auth/login",
            None,
            Some(json!({ "username": "ALEX", "password": "falsch-falsch-falsch" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid_credentials");

    let (status, body) = app
        .call(
            "POST",
            "/auth/login",
            None,
            Some(
                json!({ "username": "ALEX", "password": "ein-sicheres-passwort", "session_label": "Laptop" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let second = s(&body["token"]).to_owned();

    let (_, sessions) = app.call("GET", "/auth/sessions", Some(&second), None).await;
    let sessions = sessions.as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    let current = sessions.iter().find(|s| s["current"] == true).unwrap();
    assert_eq!(current["label"], "Laptop");
    let other_id = s(&sessions.iter().find(|s| s["current"] == false).unwrap()["id"]).to_owned();

    // Erste Session vom Laptop aus widerrufen.
    let (status, _) = app.call("DELETE", &format!("/auth/sessions/{other_id}"), Some(&second), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&first), None).await.0, StatusCode::UNAUTHORIZED);

    // Logout beendet die eigene Session.
    assert_eq!(app.call("POST", "/auth/logout", Some(&second), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&second), None).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn retention_deletes_old_messages(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (_, detail) = app.call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel" }))).await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let channel_id =
        s(&detail["channels"].as_array().unwrap().iter().find(|c| c["kind"] == "text").unwrap()["id"])
            .to_owned();

    for text in ["alt", "neu"] {
        app.call(
            "POST",
            &format!("/channels/{channel_id}/messages"),
            Some(&alex),
            Some(json!({ "content": text })),
        )
        .await;
    }
    sqlx::query("UPDATE messages SET created_at = now() - interval '10 days' WHERE content = 'alt'")
        .execute(&app.db)
        .await
        .unwrap();

    // Ohne Frist bleibt alles.
    retention::run_once(&app.db).await.unwrap();
    let (_, list) = app.call("GET", &format!("/channels/{channel_id}/messages"), Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);

    let (status, community) = app
        .call(
            "PATCH",
            &format!("/communities/{community_id}"),
            Some(&alex),
            Some(json!({ "message_retention_days": 7 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(community["message_retention_days"], 7);

    retention::run_once(&app.db).await.unwrap();
    let (_, list) = app.call("GET", &format!("/channels/{channel_id}/messages"), Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["content"], "neu");

    // null = wieder unbegrenzt.
    let (_, community) = app
        .call(
            "PATCH",
            &format!("/communities/{community_id}"),
            Some(&alex),
            Some(json!({ "message_retention_days": null })),
        )
        .await;
    assert!(community["message_retention_days"].is_null());
}

/// Das Aufräumen löscht in Portionen, bis nichts Abgelaufenes mehr übrig ist.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn retention_deletes_in_batches(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (_, detail) = app.call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel" }))).await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let channel_id =
        s(&detail["channels"].as_array().unwrap().iter().find(|c| c["kind"] == "text").unwrap()["id"])
            .to_owned();
    for i in 0..8 {
        app.call(
            "POST",
            &format!("/channels/{channel_id}/messages"),
            Some(&alex),
            Some(json!({ "content": format!("nachricht {i}") })),
        )
        .await;
    }
    // Sieben alte Nachrichten, eine bleibt jung.
    sqlx::query(
        "UPDATE messages SET created_at = now() - interval '10 days'
         WHERE content LIKE 'nachricht %' AND content <> 'nachricht 7'",
    )
    .execute(&app.db)
    .await
    .unwrap();
    app.call(
        "PATCH",
        &format!("/communities/{community_id}"),
        Some(&alex),
        Some(json!({ "message_retention_days": 7 })),
    )
    .await;

    // 7 Nachrichten in Portionen zu 3: 3 + 3 + 1.
    let deleted = retention::delete_expired_messages(&app.db, 3).await.unwrap();
    assert_eq!(deleted, 7);
    let left: Vec<String> =
        sqlx::query_scalar("SELECT content FROM messages WHERE content LIKE 'nachricht %'")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(left, ["nachricht 7"]);
    assert_eq!(retention::delete_expired_messages(&app.db, 3).await.unwrap(), 0);
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Nächstes Text-Event vom Gateway (max. 5 s).
async fn next(ws: &mut Ws) -> Value {
    let fut = async {
        loop {
            if let tungstenite::Message::Text(t) = ws.next().await.unwrap().unwrap() {
                return serde_json::from_str::<Value>(&t).unwrap();
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), fut).await.expect("kein Event binnen 5 s")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_delivers_only_visible_events(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let (_, detail) = app.call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel" }))).await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let general_id =
        s(&detail["channels"].as_array().unwrap().iter().find(|c| c["kind"] == "text").unwrap()["id"])
            .to_owned();
    let default_role_id =
        s(&detail["roles"].as_array().unwrap().iter().find(|r| r["is_default"] == true).unwrap()["id"])
            .to_owned();

    let (_, private) = app
        .call(
            "POST",
            &format!("/communities/{community_id}/channels"),
            Some(&alex),
            Some(json!({ "name": "intern", "kind": "text" })),
        )
        .await;
    let private_id = s(&private["id"]).to_owned();
    app.call(
        "PUT",
        &format!("/channels/{private_id}/overrides/{default_role_id}"),
        Some(&alex),
        Some(json!({ "allow": 0, "deny": 1 })),
    )
    .await;

    let (_, invite) =
        app.call("POST", &format!("/communities/{community_id}/invites"), Some(&alex), Some(json!({}))).await;
    let bob = app.user("bob", Some(s(&invite["code"]))).await;

    // Echten Server starten, damit eine WebSocket-Verbindung möglich ist.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    use futures_util::SinkExt;
    ws.send(tungstenite::Message::Text(json!({ "op": "identify", "token": bob }).to_string().into()))
        .await
        .unwrap();

    let ready = next(&mut ws).await;
    assert_eq!(ready["t"], "READY");
    assert_eq!(ready["d"]["community_ids"][0], community_id.as_str());

    // Nachricht im privaten Kanal darf Bob NICHT erreichen, die im öffentlichen schon.
    app.call(
        "POST",
        &format!("/channels/{private_id}/messages"),
        Some(&alex),
        Some(json!({ "content": "geheim" })),
    )
    .await;
    app.call(
        "POST",
        &format!("/channels/{general_id}/messages"),
        Some(&alex),
        Some(json!({ "content": "öffentlich" })),
    )
    .await;

    let event = next(&mut ws).await;
    assert_eq!(event["t"], "MESSAGE_CREATE");
    assert_eq!(event["d"]["content"], "öffentlich");

    // Logout schließt die Gateway-Verbindung mit 4001.
    app.call("POST", "/auth/logout", Some(&bob), None).await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Close(frame))) => return frame.map(|f| u16::from(f.code)),
                Some(Ok(_)) => continue,
                _ => return None,
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(closed, Some(4001));
}
