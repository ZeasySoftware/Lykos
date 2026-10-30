//! Integrationstests für das Freundesystem: Anfragen, Freunde, Blockieren, Datenschutz-Einstellung,
//! DMs zwischen Freunden, Gateway-Events, Export und Konto-Löschen.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::StatusCode;
use common::{PASSWORD, TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

/// `alex` (Besitzer) und `bob` teilen eine Community; `eve` hat nur ihre eigene.
struct World {
    app: TestApp,
    community: TestCommunity,
    alex: String,
    bob: String,
    eve: String,
    alex_id: String,
    bob_id: String,
    eve_id: String,
}

async fn world(db: PgPool) -> World {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Freunde").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let other = app.community(&alex, "Andere").await;
    let eve = app.invited_user(&alex, &other, "eve").await;
    let (status, _) = app.call("DELETE", &format!("/communities/{}", other.id), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (alex_id, bob_id, eve_id) =
        (app.user_id(&alex).await, app.user_id(&bob).await, app.user_id(&eve).await);
    World { app, community, alex, bob, eve, alex_id, bob_id, eve_id }
}

impl World {
    async fn request(&self, token: &str, username: &str) -> (StatusCode, Value) {
        self.app.call("POST", "/relationships", Some(token), Some(json!({ "username": username }))).await
    }

    async fn put(&self, token: &str, user_id: &str, kind: Option<&str>) -> (StatusCode, Value) {
        let body = kind.map(|k| json!({ "type": k }));
        self.app.call("PUT", &format!("/relationships/{user_id}"), Some(token), body).await
    }

    async fn remove(&self, token: &str, user_id: &str) -> StatusCode {
        self.app.call("DELETE", &format!("/relationships/{user_id}"), Some(token), None).await.0
    }

    /// Eigene Beziehungen als `(username, type)`.
    async fn list(&self, token: &str) -> Vec<(String, String)> {
        let (status, list) = self.app.call("GET", "/relationships", Some(token), None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        list.as_array()
            .unwrap()
            .iter()
            .map(|r| (s(&r["user"]["username"]).to_owned(), s(&r["type"]).to_owned()))
            .collect()
    }

    async fn befriend(&self, a: &str, b: &str, b_name: &str, a_id: &str) {
        let (status, rel) = self.request(a, b_name).await;
        assert_eq!(status, StatusCode::CREATED, "{rel}");
        let (status, rel) = self.put(b, a_id, None).await;
        assert_eq!(status, StatusCode::OK, "{rel}");
        assert_eq!(rel["type"], "friend");
    }

    async fn dm(&self, token: &str, recipients: &[&str]) -> (StatusCode, Value) {
        self.app.call("POST", "/dms", Some(token), Some(json!({ "recipient_ids": recipients }))).await
    }
}

fn pair(name: &str, kind: &str) -> (String, String) {
    (name.to_owned(), kind.to_owned())
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn request_accept_and_remove(db: PgPool) {
    let w = world(db).await;

    // Groß-/Kleinschreibung egal; Anfrage landet bei beiden Seiten.
    let (status, rel) = w.request(&w.eve, "ALEX").await;
    assert_eq!(status, StatusCode::CREATED, "{rel}");
    assert_eq!(rel["type"], "pending_out");
    assert_eq!(rel["id"], w.alex_id.as_str());
    assert_eq!(rel["user"]["username"], "alex");
    assert_eq!(w.list(&w.eve).await, [pair("alex", "pending_out")]);
    assert_eq!(w.list(&w.alex).await, [pair("eve", "pending_in")]);

    // Doppelt senden ist idempotent.
    let (status, rel) = w.request(&w.eve, "alex").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rel["type"], "pending_out");

    // Die Gegenanfrage nimmt an.
    let (status, rel) = w.request(&w.alex, "eve").await;
    assert_eq!(status, StatusCode::OK, "{rel}");
    assert_eq!(rel["type"], "friend");
    assert_eq!(w.list(&w.eve).await, [pair("alex", "friend")]);
    let (status, body) = w.request(&w.eve, "alex").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("already_friends")));

    // Freunde sehen das Profil, auch ohne gemeinsame Community.
    let (status, _) = w.app.call("GET", &format!("/users/{}", w.eve_id), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK);

    // Entfernen löscht beide Seiten; doppelt entfernen ist kein Fehler.
    assert_eq!(w.remove(&w.alex, &w.eve_id).await, StatusCode::NO_CONTENT);
    assert!(w.list(&w.eve).await.is_empty());
    assert!(w.list(&w.alex).await.is_empty());
    assert_eq!(w.remove(&w.alex, &w.eve_id).await, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn decline_and_cancel(db: PgPool) {
    let w = world(db).await;

    // Ablehnen.
    w.request(&w.bob, "alex").await;
    assert_eq!(w.remove(&w.alex, &w.bob_id).await, StatusCode::NO_CONTENT);
    assert!(w.list(&w.bob).await.is_empty());

    // Zurückziehen.
    w.request(&w.bob, "alex").await;
    assert_eq!(w.remove(&w.bob, &w.alex_id).await, StatusCode::NO_CONTENT);
    assert!(w.list(&w.alex).await.is_empty());

    // Sich selbst anfragen geht nicht; unbekannte Namen sind nicht von „nimmt nichts an“ zu unterscheiden.
    let (status, _) = w.put(&w.alex, &w.alex_id, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = w.request(&w.alex, "niemand").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("friend_request_not_allowed")));
    // Ungültige Art.
    let (status, _) = w.put(&w.alex, &w.bob_id, Some("pending_in")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn privacy_setting_limits_requests(db: PgPool) {
    let w = world(db).await;
    let (status, settings) = w.app.call("GET", "/relationships/settings", Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["friend_requests"], "everyone");

    // Nur gemeinsame Communities: Bob ja, Eve nein.
    let (status, _) = w
        .app
        .call(
            "PATCH",
            "/relationships/settings",
            Some(&w.alex),
            Some(json!({ "friend_requests": "mutual_communities" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = w.request(&w.eve, "alex").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("friend_request_not_allowed")));
    let (status, _) = w.request(&w.bob, "alex").await;
    assert_eq!(status, StatusCode::CREATED);

    // Niemand: auch Bob nicht mehr (offene Anfragen bleiben bestehen).
    w.remove(&w.bob, &w.alex_id).await;
    w.app
        .call("PATCH", "/relationships/settings", Some(&w.alex), Some(json!({ "friend_requests": "nobody" })))
        .await;
    let (status, _) = w.request(&w.bob, "alex").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Alex selbst kann weiterhin anfragen.
    let (status, _) = w.request(&w.alex, "eve").await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = w
        .app
        .call("PATCH", "/relationships/settings", Some(&w.alex), Some(json!({ "friend_requests": "alle" })))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn friends_can_dm_without_mutual_community(db: PgPool) {
    let w = world(db).await;
    let (status, _) = w.dm(&w.alex, &[&w.eve_id]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    w.befriend(&w.alex, &w.eve, "eve", &w.alex_id).await;
    // Freunde dürfen sogar mit abgeschalteten DMs schreiben.
    w.app.call("PATCH", "/dms/settings", Some(&w.eve), Some(json!({ "allow_dms": false }))).await;
    let (status, dm) = w.dm(&w.alex, &[&w.eve_id]).await;
    assert_eq!(status, StatusCode::CREATED, "{dm}");
    let (_, contacts) = w.app.call("GET", "/dms/contacts", Some(&w.alex), None).await;
    let names: Vec<&str> = contacts.as_array().unwrap().iter().map(|u| s(&u["username"])).collect();
    assert_eq!(names, ["bob", "eve"]);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn blocking(db: PgPool) {
    let w = world(db).await;
    w.befriend(&w.alex, &w.bob, "bob", &w.alex_id).await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();

    // Blockieren beendet die Freundschaft; Bob sieht nur, dass sie weg ist.
    let (status, rel) = w.put(&w.alex, &w.bob_id, Some("blocked")).await;
    assert_eq!(status, StatusCode::OK, "{rel}");
    assert_eq!(rel["type"], "blocked");
    assert_eq!(w.list(&w.alex).await, [pair("bob", "blocked")]);
    assert!(w.list(&w.bob).await.is_empty());

    // Keine Anfragen in beide Richtungen.
    let (status, body) = w.request(&w.bob, "alex").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("friend_request_not_allowed")));
    let (status, body) = w.request(&w.alex, "bob").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("user_blocked")));

    // Keine neuen DMs (auch keine Gruppen), in der bestehenden 1:1-DM nur noch lesen – beide Seiten.
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;
    let (status, _) = w.dm(&w.bob, &[&w.alex_id, &carl_id]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, contacts) = w.app.call("GET", "/dms/contacts", Some(&w.bob), None).await;
    assert!(contacts.as_array().unwrap().iter().all(|u| u["username"] != "alex"), "{contacts}");
    for token in [&w.alex, &w.bob] {
        let (status, _) = w
            .app
            .call(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(token),
                Some(json!({ "content": "hi" })),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) =
            w.app.call("GET", &format!("/channels/{channel}/messages"), Some(token), None).await;
        assert_eq!(status, StatusCode::OK);
    }

    // Blockade aufheben: wieder schreiben, aber nicht mehr befreundet.
    assert_eq!(w.remove(&w.alex, &w.bob_id).await, StatusCode::NO_CONTENT);
    assert!(w.list(&w.alex).await.is_empty());
    let (status, _) = w
        .app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&w.bob),
            Some(json!({ "content": "hi" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Gegenseitiges Blockieren: Aufheben auf einer Seite lässt die andere Blockade stehen.
    w.put(&w.alex, &w.bob_id, Some("blocked")).await;
    w.put(&w.bob, &w.alex_id, Some("blocked")).await;
    assert_eq!(w.list(&w.alex).await, [pair("bob", "blocked")]);
    w.remove(&w.alex, &w.bob_id).await;
    assert_eq!(w.list(&w.bob).await, [pair("alex", "blocked")]);
    let (status, _) = w.dm(&w.alex, &[&w.bob_id]).await;
    // Bestehende 1:1-DM wird zurückgegeben (Verlauf), schreiben geht aber nicht.
    assert_eq!(status, StatusCode::OK);
    let (status, _) = w
        .app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&w.alex),
            Some(json!({ "content": "hi" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// In einer blockierten 1:1-DM: weder eigene Nachrichten bearbeiten noch anpinnen (nur lesen).
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn blocked_dm_forbids_edit_and_pin(db: PgPool) {
    let w = world(db).await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();
    let (status, msg) = w
        .app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&w.alex),
            Some(json!({ "content": "hi" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{msg}");
    let id = s(&msg["id"]).to_owned();
    let pin = format!("/channels/{channel}/pins/{id}");
    // Vorher dürfen beide Teilnehmer anpinnen.
    assert_eq!(w.app.call("PUT", &pin, Some(&w.bob), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(w.app.call("DELETE", &pin, Some(&w.bob), None).await.0, StatusCode::NO_CONTENT);

    w.put(&w.bob, &w.alex_id, Some("blocked")).await;
    let (status, _) = w
        .app
        .call("PATCH", &format!("/messages/{id}"), Some(&w.alex), Some(json!({ "content": "geändert" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for token in [&w.alex, &w.bob] {
        assert_eq!(w.app.call("PUT", &pin, Some(token), None).await.0, StatusCode::FORBIDDEN);
    }
}

/// Anfragen/Blockieren/Entfernen (30/min) und `POST /dms` (10/min) sind pro Nutzer begrenzt.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn friend_actions_and_dms_are_rate_limited(db: PgPool) {
    use axum::{body::Body, http::Request};

    let w = world(db).await;
    let limited = async |method: &str, path: &str, token: &str, body: Value| {
        let req = Request::builder()
            .method(method)
            .uri(format!("/api{path}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let (status, headers, body) = w.app.send_full(req).await;
        (status == StatusCode::TOO_MANY_REQUESTS).then(|| {
            assert_eq!(body["error"], "rate_limited");
            headers["retry-after"].to_str().unwrap().parse::<u64>().unwrap()
        })
    };

    for _ in 0..10 {
        assert!(limited("POST", "/dms", &w.alex, json!({ "recipient_ids": [w.bob_id] })).await.is_none());
    }
    let retry = limited("POST", "/dms", &w.alex, json!({ "recipient_ids": [w.bob_id] })).await;
    assert!(retry.is_some_and(|s| s > 0 && s <= 60), "{retry:?}");
    assert!(limited("POST", "/dms", &w.bob, json!({ "recipient_ids": [w.alex_id] })).await.is_none());

    let rel = format!("/relationships/{}", w.eve_id);
    for i in 0..30 {
        let method = if i % 2 == 0 { "PUT" } else { "DELETE" };
        assert!(limited(method, &rel, &w.alex, json!({ "type": "blocked" })).await.is_none(), "{i}");
    }
    assert!(limited("PUT", &rel, &w.alex, json!({ "type": "blocked" })).await.is_some());
    assert!(limited("POST", "/relationships", &w.alex, json!({ "username": "bob" })).await.is_some());
}

/// Gesperrte Konten sind für Anfragen und neue DMs wie nicht vorhanden.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn disabled_accounts_are_unreachable(db: PgPool) {
    let w = world(db).await;
    w.befriend(&w.alex, &w.eve, "eve", &w.alex_id).await;
    let (status, _) =
        w.app.call("POST", &format!("/admin/users/{}/disable", w.bob_id), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) =
        w.app.call("POST", &format!("/admin/users/{}/disable", w.eve_id), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Gemeinsame Community (Bob) bzw. Freundschaft (Eve) helfen nicht.
    for id in [&w.bob_id, &w.eve_id] {
        let (status, body) = w.dm(&w.alex, &[id]).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::FORBIDDEN, Some("dm_not_allowed")),
            "{body}"
        );
    }
    let (status, body) = w.request(&w.alex, "bob").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("friend_request_not_allowed")));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn export_and_account_deletion(db: PgPool) {
    let w = world(db).await;
    w.befriend(&w.alex, &w.eve, "eve", &w.alex_id).await;
    w.request(&w.bob, "eve").await;

    let (status, export) = w.app.call("GET", "/auth/me/export", Some(&w.eve), None).await;
    assert_eq!(status, StatusCode::OK);
    let rels: Vec<(&str, &str)> = export["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (s(&r["user"]["username"]), s(&r["type"])))
        .collect();
    assert_eq!(rels, [("alex", "friend"), ("bob", "pending_in")]);
    assert_eq!(export["privacy"]["friend_requests"], "everyone");

    let (status, _) =
        w.app.call("DELETE", "/auth/me", Some(&w.eve), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(w.list(&w.alex).await.is_empty());
    assert!(w.list(&w.bob).await.is_empty());
}

// --- Gateway ------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn connect(addr: SocketAddr, token: &str) -> (Ws, Value) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    ws.send(tungstenite::Message::Text(json!({ "op": "identify", "token": token }).to_string().into()))
        .await
        .unwrap();
    let ready = next_of(&mut ws, "READY").await;
    (ws, ready)
}

async fn try_next(ws: &mut Ws, wait: Duration) -> Option<Value> {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => return serde_json::from_str::<Value>(&t).unwrap(),
                Some(Ok(_)) => continue,
                other => panic!("Verbindung beendet: {other:?}"),
            }
        }
    };
    tokio::time::timeout(wait, fut).await.ok()
}

async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let event = try_next(ws, Duration::from_secs(5)).await.expect("kein Event binnen 5 s");
        if event["t"] == kind {
            return event["d"].clone();
        }
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_events_and_presence_for_friends(db: PgPool) {
    let w = world(db).await;
    let addr = serve(&w.app).await;
    let (mut alex_ws, _) = connect(addr, &w.alex).await;
    let (mut eve_ws, ready) = connect(addr, &w.eve).await;
    // Ohne gemeinsame Community und ohne Freundschaft kennt Eve Alex' Status nicht.
    assert!(ready["presences"].as_array().unwrap().iter().all(|p| p["user_id"] != w.alex_id.as_str()));

    // Anfrage: beide Seiten bekommen RELATIONSHIP_ADD mit ihrer Sicht.
    w.request(&w.eve, "alex").await;
    let add = next_of(&mut alex_ws, "RELATIONSHIP_ADD").await;
    assert_eq!((add["type"].as_str(), add["id"].as_str()), (Some("pending_in"), Some(w.eve_id.as_str())));
    let add = next_of(&mut eve_ws, "RELATIONSHIP_ADD").await;
    assert_eq!(add["type"], "pending_out");

    // Annehmen: Freundschaft + gegenseitiger Status.
    w.put(&w.alex, &w.eve_id, None).await;
    let add = next_of(&mut eve_ws, "RELATIONSHIP_ADD").await;
    assert_eq!(add["type"], "friend");
    let add = next_of(&mut alex_ws, "RELATIONSHIP_ADD").await;
    assert_eq!(add["type"], "friend");
    let presence = next_of(&mut eve_ws, "PRESENCE_UPDATE").await;
    assert_eq!(
        (presence["user_id"].as_str(), presence["status"].as_str()),
        (Some(w.alex_id.as_str()), Some("online"))
    );

    // Statuswechsel erreicht Freunde auch ohne gemeinsame Community.
    alex_ws
        .send(tungstenite::Message::Text(json!({ "op": "presence", "status": "dnd" }).to_string().into()))
        .await
        .unwrap();
    let presence = next_of(&mut eve_ws, "PRESENCE_UPDATE").await;
    assert_eq!(presence["status"], "dnd");

    // Neue Verbindung: READY enthält Freunde.
    let (_eve2, ready) = connect(addr, &w.eve).await;
    assert!(ready["presences"].as_array().unwrap().iter().any(|p| p["user_id"] == w.alex_id.as_str()));

    // Profiländerungen gehen an Freunde.
    w.app.call("PATCH", "/auth/me", Some(&w.alex), Some(json!({ "display_name": "Alex W." }))).await;
    let update = next_of(&mut eve_ws, "USER_UPDATE").await;
    assert_eq!(update["display_name"], "Alex W.");

    // Blockieren: Eve bekommt nur RELATIONSHIP_REMOVE.
    w.put(&w.alex, &w.eve_id, Some("blocked")).await;
    let removed = next_of(&mut eve_ws, "RELATIONSHIP_REMOVE").await;
    assert_eq!(removed["id"], w.alex_id.as_str());
    let add = next_of(&mut alex_ws, "RELATIONSHIP_ADD").await;
    assert_eq!(add["type"], "blocked");
}

/// Liefert `kind` binnen `wait` – andere Events werden übersprungen.
async fn event_within(ws: &mut Ws, kind: &str, wait: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = try_next(ws, left).await?;
        if event["t"] == kind {
            return Some(event["d"].clone());
        }
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn own_request_or_block_does_not_reveal_profile(db: PgPool) {
    let w = world(db).await;
    let addr = serve(&w.app).await;
    let (mut eve_ws, _) = connect(addr, &w.eve).await;
    let profile =
        async |token: &str, id: &str| w.app.call("GET", &format!("/users/{id}"), Some(token), None).await.0;

    // Eve fragt Alex (keine gemeinsame Community) an: Alex sieht Eves Profil, Eve das von Alex nicht.
    let (status, rel) = w.request(&w.eve, "alex").await;
    assert_eq!(status, StatusCode::CREATED, "{rel}");
    assert_eq!(rel["user"]["username"], "alex");
    assert_eq!(profile(&w.alex, &w.eve_id).await, StatusCode::OK);
    assert_eq!(profile(&w.eve, &w.alex_id).await, StatusCode::NOT_FOUND);
    // … und bekommt auch keine Profiländerungen von Alex.
    w.app.call("PATCH", "/auth/me", Some(&w.alex), Some(json!({ "display_name": "Alex W." }))).await;
    assert!(event_within(&mut eve_ws, "USER_UPDATE", Duration::from_millis(500)).await.is_none());

    // Blockieren öffnet das Profil ebenfalls nicht – in keine Richtung.
    w.remove(&w.eve, &w.alex_id).await;
    let (status, _) = w.put(&w.eve, &w.alex_id, Some("blocked")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile(&w.eve, &w.alex_id).await, StatusCode::NOT_FOUND);
    assert_eq!(profile(&w.alex, &w.eve_id).await, StatusCode::NOT_FOUND);
    w.app.call("PATCH", "/auth/me", Some(&w.alex), Some(json!({ "display_name": "Alex X." }))).await;
    assert!(event_within(&mut eve_ws, "USER_UPDATE", Duration::from_millis(500)).await.is_none());

    // Alex fragt Eve an (nach Aufheben der Blockade): jetzt sieht Eve das Profil samt Änderungen.
    w.remove(&w.eve, &w.alex_id).await;
    let (status, _) = w.request(&w.alex, "eve").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(profile(&w.eve, &w.alex_id).await, StatusCode::OK);
    w.app.call("PATCH", "/auth/me", Some(&w.alex), Some(json!({ "display_name": "Alex Y." }))).await;
    let update = event_within(&mut eve_ws, "USER_UPDATE", Duration::from_secs(5)).await.expect("USER_UPDATE");
    assert_eq!(update["display_name"], "Alex Y.");
}
