//! Integrationstests für Direktnachrichten und Nachrichtensuche (MVP 2, Paket C).

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use lykos_server::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

/// Drei Nutzer: `alex` (Besitzer) und `bob` in einer Community, `eve` in einer eigenen.
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
    world_with(TestApp::new(db)).await
}

async fn world_with(app: TestApp) -> World {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Freunde").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    // Eve kommt über eine Einladung auf die Instanz; die Community wird danach gelöscht und Eve hat
    // nur ihre eigene – sie teilt keine Community mit Alex/Bob.
    let other = app.community(&alex, "Andere").await;
    let eve = app.invited_user(&alex, &other, "eve").await;
    let (status, _) = app.call("DELETE", &format!("/communities/{}", other.id), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    app.community(&eve, "Eves Ecke").await;
    let (alex_id, bob_id, eve_id) =
        (app.user_id(&alex).await, app.user_id(&bob).await, app.user_id(&eve).await);
    World { app, community, alex, bob, eve, alex_id, bob_id, eve_id }
}

impl World {
    async fn dm(&self, token: &str, recipients: &[&str]) -> (StatusCode, Value) {
        self.app.call("POST", "/dms", Some(token), Some(json!({ "recipient_ids": recipients }))).await
    }

    async fn send(&self, token: &str, channel: &str, content: &str) -> Value {
        let (status, msg) = self
            .app
            .call(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(token),
                Some(json!({ "content": content })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{msg}");
        msg
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn one_to_one_dm_is_created_and_reused(db: PgPool) {
    let w = world(db).await;

    let (status, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert_eq!(status, StatusCode::CREATED, "{dm}");
    assert_eq!(dm["kind"], "dm");
    assert_eq!(dm["name"], Value::Null);
    let names: Vec<&str> = dm["recipients"].as_array().unwrap().iter().map(|u| s(&u["username"])).collect();
    assert_eq!(names, ["alex", "bob"]);
    let channel = s(&dm["id"]).to_owned();

    // Erneut eröffnen (auch von der Gegenseite) liefert dieselbe DM.
    let (status, again) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], dm["id"]);
    let (status, reverse) = w.dm(&w.bob, &[&w.alex_id]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reverse["id"], dm["id"]);

    // Nachrichten wie in Textkanälen; `community_id` ist null.
    let msg = w.send(&w.alex, &channel, "Hallo Bob").await;
    assert_eq!(msg["community_id"], Value::Null);
    let (status, list) =
        w.app.call("GET", &format!("/channels/{channel}/messages"), Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["content"], "Hallo Bob");

    // Liste mit neuester Nachricht und Lesestand.
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.bob), None).await;
    assert_eq!(dms.as_array().unwrap().len(), 1);
    assert_eq!(dms[0]["last_message_id"], msg["id"]);
    assert_eq!(dms[0]["last_read_message_id"], Value::Null);
    let (status, _) = w
        .app
        .call(
            "PUT",
            &format!("/channels/{channel}/read"),
            Some(&w.bob),
            Some(json!({ "message_id": msg["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.bob), None).await;
    assert_eq!(dms[0]["last_read_message_id"], msg["id"]);

    // 1:1-DMs kann man nicht verlassen und nicht umbenennen.
    let (status, body) = w.app.call("DELETE", &format!("/dms/{channel}"), Some(&w.bob), None).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "cannot_leave_dm"));
    let (status, _) =
        w.app.call("PATCH", &format!("/dms/{channel}"), Some(&w.bob), Some(json!({ "name": "x" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn strangers_cannot_access_dms(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();
    let msg = w.send(&w.alex, &channel, "geheim").await;
    let msg_id = s(&msg["id"]);

    // Carl teilt die Community, ist aber kein Teilnehmer: alles 404 (Existenz nicht verraten).
    for (method, path, body) in [
        ("GET", format!("/channels/{channel}/messages"), None),
        ("POST", format!("/channels/{channel}/messages"), Some(json!({ "content": "hi" }))),
        ("PUT", format!("/channels/{channel}/read"), Some(json!({ "message_id": msg_id }))),
        ("PATCH", format!("/messages/{msg_id}"), Some(json!({ "content": "x" }))),
        ("DELETE", format!("/messages/{msg_id}"), None),
        ("PATCH", format!("/dms/{channel}"), Some(json!({ "name": "x" }))),
        ("DELETE", format!("/dms/{channel}"), None),
    ] {
        let (status, _) = w.app.call(method, &path, Some(&carl), body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    let (_, dms) = w.app.call("GET", "/dms", Some(&carl), None).await;
    assert_eq!(dms, json!([]));

    // Teilnehmer dürfen keine fremden Nachrichten löschen und keine Kanal-Verwaltung nutzen.
    let (status, _) = w.app.call("DELETE", &format!("/messages/{msg_id}"), Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = w
        .app
        .call("PATCH", &format!("/channels/{channel}"), Some(&w.bob), Some(json!({ "name": "x" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = w.app.call("DELETE", &format!("/channels/{channel}"), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = w.app.call("GET", &format!("/channels/{channel}/overrides"), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let path = format!("/channels/{channel}/overrides/{}", w.community.default_role);
    let (status, _) = w.app.call("PUT", &path, Some(&w.alex), Some(json!({ "allow": 0, "deny": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Voice gibt es in DMs (noch) nicht.
    let (status, _) =
        w.app.call("POST", &format!("/channels/{channel}/voice/join"), Some(&w.bob), None).await;
    assert_ne!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn dm_requires_shared_community_and_consent(db: PgPool) {
    let w = world(db).await;

    // Keine gemeinsame Community.
    let (status, body) = w.dm(&w.alex, &[&w.eve_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    let (status, _) = w.dm(&w.eve, &[&w.bob_id]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Unbekannte Nutzer sind davon nicht zu unterscheiden.
    let (status, body) = w.dm(&w.alex, &[&uuid::Uuid::now_v7().to_string()]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    // Sich selbst und leere Listen gehen nicht.
    let (status, _) = w.dm(&w.alex, &[&w.alex_id]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = w.dm(&w.alex, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Kontakte: nur Mitglieder gemeinsamer Communities.
    let (_, contacts) = w.app.call("GET", "/dms/contacts", Some(&w.alex), None).await;
    let names: Vec<&str> = contacts.as_array().unwrap().iter().map(|u| s(&u["username"])).collect();
    assert_eq!(names, ["bob"]);

    // Bob schaltet DMs ab → keine neuen DMs, auch nicht in Kontakten.
    let (_, settings) = w.app.call("GET", "/dms/settings", Some(&w.bob), None).await;
    assert_eq!(settings["allow_dms"], true);
    let (status, _) =
        w.app.call("PATCH", "/dms/settings", Some(&w.bob), Some(json!({ "allow_dms": false }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    let (_, contacts) = w.app.call("GET", "/dms/contacts", Some(&w.alex), None).await;
    assert_eq!(contacts, json!([]));

    // Bob selbst darf weiterhin schreiben; danach bleibt die DM für beide nutzbar.
    let (status, dm) = w.dm(&w.bob, &[&w.alex_id]).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, again) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert_eq!((status, &again["id"]), (StatusCode::OK, &dm["id"]));
    w.send(&w.alex, s(&dm["id"]), "trotzdem erreichbar").await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn group_dm_rename_and_leave(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;

    let (status, dm) = w
        .app
        .call(
            "POST",
            "/dms",
            Some(&w.alex),
            Some(json!({ "recipient_ids": [w.bob_id, carl_id], "name": " Spieleabend " })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{dm}");
    assert_eq!(dm["kind"], "group_dm");
    assert_eq!(dm["name"], "Spieleabend");
    assert_eq!(dm["recipients"].as_array().unwrap().len(), 3);
    let channel = s(&dm["id"]).to_owned();

    // Gruppen werden nicht wiederverwendet.
    let (_, second) = w
        .app
        .call("POST", "/dms", Some(&w.alex), Some(json!({ "recipient_ids": [w.bob_id, carl_id] })))
        .await;
    assert_ne!(second["id"], dm["id"]);
    assert_eq!(second["name"], Value::Null);

    // Jeder Teilnehmer darf umbenennen.
    let (status, renamed) =
        w.app.call("PATCH", &format!("/dms/{channel}"), Some(&w.bob), Some(json!({ "name": "LAN" }))).await;
    assert_eq!((status, &renamed["name"]), (StatusCode::OK, &json!("LAN")));
    let (_, cleared) =
        w.app.call("PATCH", &format!("/dms/{channel}"), Some(&w.bob), Some(json!({ "name": null }))).await;
    assert_eq!(cleared["name"], Value::Null);

    // Carl verlässt die Gruppe und verliert den Zugriff.
    w.send(&carl, &channel, "tschüss").await;
    let (status, _) = w.app.call("DELETE", &format!("/dms/{channel}"), Some(&carl), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = w.app.call("GET", &format!("/channels/{channel}/messages"), Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.alex), None).await;
    let group = dms.as_array().unwrap().iter().find(|d| d["id"] == channel.as_str()).unwrap();
    assert_eq!(group["recipients"].as_array().unwrap().len(), 2);

    // Verlassen alle, verschwindet der Kanal.
    w.app.call("DELETE", &format!("/dms/{channel}"), Some(&w.alex), None).await;
    w.app.call("DELETE", &format!("/dms/{channel}"), Some(&w.bob), None).await;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM channels WHERE id = $1::uuid)")
        .bind(&channel)
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert!(!exists);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn group_dm_size_is_limited(db: PgPool) {
    let w = world(db).await;
    let mut ids = Vec::new();
    for i in 0..10 {
        let t = w.app.invited_user(&w.alex, &w.community, &format!("user{i}")).await;
        ids.push(w.app.user_id(&t).await);
    }
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (status, body) = w.dm(&w.alex, &refs).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_recipients"));
    let (status, _) = w.dm(&w.alex, &refs[..9]).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn dm_attachments_are_private(db: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::for_tests();
    config.storage_dir = dir.path().to_path_buf();
    let w = world_with(TestApp::with_config(db, config)).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();

    let boundary = "lykos-dm-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notiz.txt\"\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"nur fuer bob");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let upload = |token: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel}/attachments"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body.clone()))
            .unwrap()
    };

    let (status, _) = w.app.send(upload(&carl)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, attachment) = w.app.send(upload(&w.alex)).await;
    assert_eq!(status, StatusCode::CREATED, "{attachment}");
    let (status, msg) = w
        .app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&w.alex),
            Some(json!({ "content": "", "attachment_ids": [attachment["id"]] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{msg}");
    let url = s(&msg["attachments"][0]["url"]).trim_start_matches("/api").to_owned();

    let (status, content) = w.app.call("GET", &url, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content, "nur fuer bob");
    let (status, _) = w.app.call("GET", &url, Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn messages_around_an_id(db: PgPool) {
    let w = world(db).await;
    let channel = w.community.text_channel.clone();
    let mut ids = Vec::new();
    for i in 0..20 {
        ids.push(s(&w.send(&w.alex, &channel, &format!("n{i}")).await["id"]).to_owned());
    }
    let path = format!("/channels/{channel}/messages?around={}&limit=6", ids[10]);
    let (status, page) = w.app.call("GET", &path, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    let contents: Vec<&str> = page.as_array().unwrap().iter().map(|m| s(&m["content"])).collect();
    assert_eq!(contents, ["n13", "n12", "n11", "n10", "n9", "n8"]);

    // Am Ende des Verlaufs füllen ältere Nachrichten auf.
    let path = format!("/channels/{channel}/messages?around={}&limit=6", ids[19]);
    let (_, page) = w.app.call("GET", &path, Some(&w.bob), None).await;
    let contents: Vec<&str> = page.as_array().unwrap().iter().map(|m| s(&m["content"])).collect();
    assert_eq!(contents, ["n19", "n18", "n17", "n16", "n15", "n14"]);

    // Nachladen nach dem Sprung: die nächsten neueren, neueste zuerst.
    let path = format!("/channels/{channel}/messages?after={}&limit=3", ids[13]);
    let (status, page) = w.app.call("GET", &path, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    let contents: Vec<&str> = page.as_array().unwrap().iter().map(|m| s(&m["content"])).collect();
    assert_eq!(contents, ["n16", "n15", "n14"]);

    let path = format!("/channels/{channel}/messages?around={}&before={}", ids[10], ids[11]);
    let (status, _) = w.app.call("GET", &path, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// --- Suche --------------------------------------------------------------------

fn contents(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|m| s(&m["content"])).collect()
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn community_search_respects_visibility(db: PgPool) {
    let w = world(db).await;
    let general = w.community.text_channel.clone();

    // Privater Kanal: Standardrolle sieht ihn nicht.
    let (_, private) = w
        .app
        .call(
            "POST",
            &format!("/communities/{}/channels", w.community.id),
            Some(&w.alex),
            Some(json!({ "name": "mods", "kind": "text" })),
        )
        .await;
    let private = s(&private["id"]).to_owned();
    let path = format!("/channels/{private}/overrides/{}", w.community.default_role);
    let (status, _) = w.app.call("PUT", &path, Some(&w.alex), Some(json!({ "allow": 0, "deny": 1 }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    w.send(&w.alex, &general, "Treffen wir uns im Gartenhaus?").await;
    w.send(&w.bob, &general, "Die Häuser am See sind schön").await;
    w.send(&w.alex, &private, "Geheimes Haus der Moderation").await;
    w.send(&w.bob, &general, "völlig anderes Thema").await;

    let search = |token: String, query: String| {
        let app = &w.app;
        let path = format!("/communities/{}/search?{query}", w.community.id);
        async move { app.call("GET", &path, Some(&token), None).await }
    };

    // Bob sieht den privaten Kanal nicht – auch nicht über die Suche. Deutscher Stamm: „Haus“ ↔ „Häuser“.
    let (status, hits) = search(w.bob.clone(), "q=haus".into()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(contents(&hits), ["Die Häuser am See sind schön"]);
    let (_, hits) = search(w.bob.clone(), format!("q=haus&channel_id={private}")).await;
    assert_eq!(hits, json!([]));

    // Alex sieht alles; Präfix findet „Gartenhaus“ über `garten`.
    let (_, hits) = search(w.alex.clone(), "q=haus".into()).await;
    assert_eq!(contents(&hits), ["Geheimes Haus der Moderation", "Die Häuser am See sind schön"]);
    let (_, hits) = search(w.alex.clone(), "q=garten".into()).await;
    assert_eq!(contents(&hits), ["Treffen wir uns im Gartenhaus?"]);
    // Stoppwörter findet der `simple`-Rückfall.
    let (_, hits) = search(w.alex.clone(), "q=die".into()).await;
    assert_eq!(contents(&hits), ["Die Häuser am See sind schön"]);

    // Filter: Autor, Kanal, Blättern.
    let (_, hits) = search(w.alex.clone(), format!("q=haus&author_id={}", w.bob_id)).await;
    assert_eq!(contents(&hits), ["Die Häuser am See sind schön"]);
    let (_, all) = search(w.alex.clone(), "q=haus&limit=1".into()).await;
    assert_eq!(contents(&all), ["Geheimes Haus der Moderation"]);
    let (_, next) = search(w.alex.clone(), format!("q=haus&limit=1&before={}", s(&all[0]["id"]))).await;
    assert_eq!(contents(&next), ["Die Häuser am See sind schön"]);

    // Ungültige Suchen und fremde Communities.
    let (status, _) = search(w.alex.clone(), "q=%21%21".into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = search(w.eve.clone(), "q=haus".into()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn dm_search_only_covers_own_dms(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;
    let (_, ab) = w.dm(&w.alex, &[&w.bob_id]).await;
    let (_, ac) = w.dm(&w.alex, &[&carl_id]).await;
    w.send(&w.alex, s(&ab["id"]), "Pizza heute?").await;
    w.send(&w.alex, s(&ac["id"]), "Pizza morgen?").await;
    w.send(&w.alex, &w.community.text_channel, "Pizza für alle").await;

    let (status, hits) = w.app.call("GET", "/dms/search?q=pizza", Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(contents(&hits), ["Pizza heute?"]);
    let (_, hits) = w.app.call("GET", "/dms/search?q=pizza", Some(&w.alex), None).await;
    assert_eq!(contents(&hits), ["Pizza morgen?", "Pizza heute?"]);
    let (_, hits) = w
        .app
        .call("GET", &format!("/dms/search?q=pizza&channel_id={}", s(&ac["id"])), Some(&w.bob), None)
        .await;
    assert_eq!(hits, json!([]));
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

/// Keine Events der genannten Typen binnen `wait`.
async fn assert_none_of(ws: &mut Ws, kinds: &[&str], wait: Duration) {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return;
        }
        if let Some(event) = try_next(ws, left).await {
            assert!(!kinds.contains(&s(&event["t"])), "unerwartetes Event: {event}");
        }
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_delivers_dm_events_to_participants_only(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let addr = serve(&w.app).await;
    let (mut bob_ws, ready) = connect(addr, &w.bob).await;
    assert_eq!(ready["dm_channel_ids"], json!([]));
    let (mut carl_ws, _) = connect(addr, &carl).await;

    // Neue DM: Bob erfährt davon und abonniert sie sofort.
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();
    let created = next_of(&mut bob_ws, "DM_CREATE").await;
    assert_eq!(created["id"], dm["id"]);

    w.send(&w.alex, &channel, "psst").await;
    let msg = next_of(&mut bob_ws, "MESSAGE_CREATE").await;
    assert_eq!((msg["content"].as_str(), &msg["community_id"]), (Some("psst"), &Value::Null));

    // Tippen in der DM.
    let (mut alex_ws, ready) = connect(addr, &w.alex).await;
    assert_eq!(ready["dm_channel_ids"], json!([channel]));
    alex_ws
        .send(tungstenite::Message::Text(json!({ "op": "typing", "channel_id": channel }).to_string().into()))
        .await
        .unwrap();
    let typing = next_of(&mut bob_ws, "TYPING_START").await;
    assert_eq!(typing["channel_id"], channel.as_str());
    assert_eq!(typing["community_id"], Value::Null);

    // Lesestand wird an die eigenen Verbindungen gemeldet.
    let (_, _) = w
        .app
        .call(
            "PUT",
            &format!("/channels/{channel}/read"),
            Some(&w.bob),
            Some(json!({ "message_id": msg["id"] })),
        )
        .await;
    let read = next_of(&mut bob_ws, "READ_STATE_UPDATE").await;
    assert_eq!(read["community_id"], Value::Null);

    // Carl bekommt nichts davon mit.
    assert_none_of(
        &mut carl_ws,
        &["DM_CREATE", "MESSAGE_CREATE", "TYPING_START"],
        Duration::from_millis(500),
    )
    .await;

    // Gruppe verlassen: DM_DELETE an den Verlassenden, danach keine Nachrichten mehr.
    let carl_id = w.app.user_id(&carl).await;
    let (_, group) = w
        .app
        .call("POST", "/dms", Some(&w.alex), Some(json!({ "recipient_ids": [w.bob_id, carl_id] })))
        .await;
    let group_id = s(&group["id"]).to_owned();
    next_of(&mut carl_ws, "DM_CREATE").await;
    w.app.call("DELETE", &format!("/dms/{group_id}"), Some(&carl), None).await;
    let deleted = next_of(&mut carl_ws, "DM_DELETE").await;
    assert_eq!(deleted["id"], group_id.as_str());
    let update = next_of(&mut bob_ws, "DM_UPDATE").await;
    assert_eq!(update["recipients"].as_array().unwrap().len(), 2);
    w.send(&w.alex, &group_id, "ohne Carl").await;
    next_of(&mut bob_ws, "MESSAGE_CREATE").await;
    assert_none_of(&mut carl_ws, &["MESSAGE_CREATE"], Duration::from_millis(500)).await;
}

// --- Zusammenspiel mit Paket A (Reaktionen, Pins, Erwähnungen) und B (Profile, Konto löschen) ---

fn enc(segment: &str) -> String {
    segment.bytes().map(|b| format!("%{b:02X}")).collect()
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reactions_pins_and_mentions_in_dms(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();
    let msg = w.send(&w.alex, &channel, "Hallo @bob und @carl und @everyone").await;
    let id = s(&msg["id"]).to_owned();

    // Erwähnungen nur für Teilnehmer, `@everyone` wird in DMs ignoriert.
    assert_eq!(msg["mentions"], json!([w.bob_id]));
    assert_eq!(msg["mention_everyone"], false);
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.bob), None).await;
    assert_eq!(dms[0]["mention_count"], 1);

    // Reaktionen: Teilnehmer ja, Fremde nicht.
    let reaction = format!("/messages/{id}/reactions/{}/@me", enc("👍"));
    let (status, _) = w.app.call("PUT", &reaction, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = w.app.call("PUT", &reaction, Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = w.app.call("GET", &format!("/channels/{channel}/messages"), Some(&w.bob), None).await;
    assert_eq!(list[0]["reactions"][0]["count"], 1);
    assert_eq!(list[0]["reactions"][0]["me"], true);

    // Antworten.
    let (status, reply) = w
        .app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&w.bob),
            Some(json!({ "content": "Hi!", "reply_to_id": id })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");

    // Anpinnen dürfen alle Teilnehmer (ohne MANAGE_MESSAGES), ohne Audit-Eintrag.
    let pin = format!("/channels/{channel}/pins/{id}");
    let (status, _) = w.app.call("PUT", &pin, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, pins) = w.app.call("GET", &format!("/channels/{channel}/pins"), Some(&w.alex), None).await;
    assert_eq!(pins.as_array().unwrap().len(), 1);
    let (status, _) = w.app.call("PUT", &pin, Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action LIKE 'message.%pin'")
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(audits, 0);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn dm_partners_see_each_others_profile(db: PgPool) {
    let w = world(db).await;
    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert!(dm["recipients"][0].as_object().unwrap().contains_key("avatar_url"));

    // Bob verlässt die einzige gemeinsame Community – über die DM bleibt das Profil sichtbar.
    let (status, _) = w
        .app
        .call("DELETE", &format!("/communities/{}/members/{}", w.community.id, w.bob_id), Some(&w.bob), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, profile) = w.app.call("GET", &format!("/users/{}", w.bob_id), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["username"], "bob");
    // Eve teilt weder Community noch DM.
    let (status, _) = w.app.call("GET", &format!("/users/{}", w.bob_id), Some(&w.eve), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn deleting_an_account_cleans_up_dms(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;

    // Bob hat eine 1:1-DM mit Alex, eine Gruppe mit Alex und Carl und eine Gruppe, in der nur er übrig ist.
    let (_, direct) = w.dm(&w.bob, &[&w.alex_id]).await;
    let (_, group) = w.dm(&w.bob, &[&w.alex_id, &carl_id]).await;
    let (_, lonely) = w.dm(&w.bob, &[&w.alex_id, &carl_id]).await;
    w.app.call("DELETE", &format!("/dms/{}", s(&lonely["id"])), Some(&w.alex), None).await;
    w.app.call("DELETE", &format!("/dms/{}", s(&lonely["id"])), Some(&carl), None).await;
    w.send(&w.bob, s(&direct["id"]), "bis bald").await;
    w.send(&w.bob, s(&group["id"]), "tschüss").await;

    let (status, body) = w
        .app
        .call(
            "DELETE",
            "/auth/me",
            Some(&w.bob),
            Some(json!({ "password": common::PASSWORD, "delete_messages": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // Verbleibende Teilnehmer behalten ihre DMs, Bob ist daraus verschwunden.
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.alex), None).await;
    assert_eq!(dms.as_array().unwrap().len(), 2);
    for dm in dms.as_array().unwrap() {
        assert!(dm["recipients"].as_array().unwrap().iter().all(|u| u["id"] != w.bob_id.as_str()));
    }
    let (_, list) =
        w.app.call("GET", &format!("/channels/{}/messages", s(&group["id"])), Some(&carl), None).await;
    assert_eq!(list, json!([]));
    // Die leere Gruppe ist samt Kanal weg.
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM channels WHERE id = $1::uuid)")
        .bind(s(&lonely["id"]))
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert!(!exists);
}

// --- Anpinnen, Leute hinzufügen, Gruppen explizit erstellen (Migration 0022) ---

impl World {
    async fn group(&self, token: &str, recipients: &[&str], name: Option<&str>) -> (StatusCode, Value) {
        self.app
            .call(
                "POST",
                "/dms",
                Some(token),
                Some(json!({ "recipient_ids": recipients, "name": name, "group": true })),
            )
            .await
    }

    async fn add(&self, token: &str, channel: &str, recipients: &[&str]) -> (StatusCode, Value) {
        self.app
            .call(
                "POST",
                &format!("/dms/{channel}/recipients"),
                Some(token),
                Some(json!({ "recipient_ids": recipients })),
            )
            .await
    }

    async fn block(&self, token: &str, user_id: &str) {
        let (status, body) = self
            .app
            .call(
                "PUT",
                &format!("/relationships/{user_id}"),
                Some(token),
                Some(json!({ "type": "blocked" })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

fn find<'a>(dms: &'a Value, id: &Value) -> &'a Value {
    dms.as_array().unwrap().iter().find(|d| &d["id"] == id).expect("DM fehlt in der Liste")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn group_flag_creates_groups_with_one_or_no_recipient(db: PgPool) {
    let w = world(db).await;

    // Ohne Flag bleibt es bei der 1:1-DM.
    let (status, direct) = w.dm(&w.alex, &[&w.bob_id]).await;
    assert_eq!((status, s(&direct["kind"])), (StatusCode::CREATED, "dm"));

    // Mit Flag: immer eine neue Gruppe, auch mit nur einer weiteren Person – mit Namen.
    let (status, group) = w.group(&w.alex, &[&w.bob_id], Some(" Duo ")).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    assert_eq!((s(&group["kind"]), s(&group["name"])), ("group_dm", "Duo"));
    assert_eq!(group["recipients"].as_array().unwrap().len(), 2);
    assert_eq!(group["pinned_at"], Value::Null);
    let (_, again) = w.group(&w.alex, &[&w.bob_id], None).await;
    assert_ne!(again["id"], group["id"]);
    assert_ne!(group["id"], direct["id"]);

    // Ganz allein (Leute kommen später dazu).
    let (status, alone) = w.group(&w.alex, &[], Some("Später")).await;
    assert_eq!(status, StatusCode::CREATED, "{alone}");
    assert_eq!(alone["recipients"].as_array().unwrap().len(), 1);
    w.send(&w.alex, s(&alone["id"]), "Notiz an mich").await;

    // Erreichbarkeit gilt auch hier.
    let (status, body) = w.group(&w.alex, &[&w.eve_id], None).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    // Ohne Flag bleibt eine leere Liste ungültig.
    let (status, _) = w.dm(&w.alex, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn pins_are_personal_and_limited(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let addr = serve(&w.app).await;
    let (mut alex_ws, _) = connect(addr, &w.alex).await;
    let (mut bob_ws, _) = connect(addr, &w.bob).await;

    let (_, dm) = w.dm(&w.alex, &[&w.bob_id]).await;
    let channel = s(&dm["id"]).to_owned();
    let pin = format!("/dms/{channel}/pin");

    let (status, pinned) = w.app.call("PUT", &pin, Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    let at = pinned["pinned_at"].clone();
    assert!(at.is_string());
    // Alle eigenen Sitzungen erfahren davon, das Gegenüber nicht.
    let ev = next_of(&mut alex_ws, "DM_PIN_UPDATE").await;
    assert_eq!(ev, json!({ "channel_id": channel, "pinned_at": at }));
    assert_none_of(&mut bob_ws, &["DM_PIN_UPDATE"], Duration::from_millis(300)).await;

    // Eigene Sicht: Bob sieht keinen Pin.
    let (_, alex_dms) = w.app.call("GET", "/dms", Some(&w.alex), None).await;
    assert_eq!(find(&alex_dms, &dm["id"])["pinned_at"], at);
    let (_, bob_dms) = w.app.call("GET", "/dms", Some(&w.bob), None).await;
    assert_eq!(find(&bob_dms, &dm["id"])["pinned_at"], Value::Null);

    // Erneut anpinnen behält den Zeitpunkt.
    let (status, again) = w.app.call("PUT", &pin, Some(&w.alex), None).await;
    assert_eq!((status, &again["pinned_at"]), (StatusCode::OK, &at));

    // Fremde DM: 404.
    let (status, _) = w.app.call("PUT", &pin, Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = w.app.call("DELETE", &pin, Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) =
        w.app.call("PUT", &format!("/dms/{}/pin", uuid::Uuid::now_v7()), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Lösen.
    let (status, _) = w.app.call("DELETE", &pin, Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ev = next_of(&mut alex_ws, "DM_PIN_UPDATE").await;
    assert_eq!(ev, json!({ "channel_id": channel, "pinned_at": null }));
    let (_, alex_dms) = w.app.call("GET", "/dms", Some(&w.alex), None).await;
    assert_eq!(find(&alex_dms, &dm["id"])["pinned_at"], Value::Null);
    let (status, _) = w.app.call("DELETE", &pin, Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Obergrenze: 20 angepinnte Gruppen (direkt angelegt, am Rate-Limit vorbei), die 21. geht nicht.
    let alex_id: uuid::Uuid = w.alex_id.parse().unwrap();
    let mut first = None;
    for _ in 0..20 {
        let id = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO channels (id, community_id, name, kind) VALUES ($1, NULL, '', 'group_dm')")
            .bind(id)
            .execute(&w.app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO dm_recipients (channel_id, user_id, pinned_at) VALUES ($1, $2, now())")
            .bind(id)
            .bind(alex_id)
            .execute(&w.app.db)
            .await
            .unwrap();
        first.get_or_insert(id);
    }
    let (status, body) = w.app.call("PUT", &pin, Some(&w.alex), None).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_dm_pins"));
    // Schon angepinnte bleiben idempotent, nach dem Lösen einer anderen geht es wieder.
    let first = first.unwrap();
    let (status, _) = w.app.call("PUT", &format!("/dms/{first}/pin"), Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK);
    w.app.call("DELETE", &format!("/dms/{first}/pin"), Some(&w.alex), None).await;
    let (status, _) = w.app.call("PUT", &pin, Some(&w.alex), None).await;
    assert_eq!(status, StatusCode::OK);
    // Bob ist davon unberührt.
    let (status, _) = w.app.call("PUT", &pin, Some(&w.bob), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn adding_to_a_one_to_one_dm_creates_a_group(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;
    let addr = serve(&w.app).await;
    let (mut bob_ws, _) = connect(addr, &w.bob).await;
    let (mut carl_ws, _) = connect(addr, &carl).await;

    let (_, direct) = w.dm(&w.alex, &[&w.bob_id]).await;
    let direct_id = s(&direct["id"]).to_owned();
    next_of(&mut bob_ws, "DM_CREATE").await;
    w.send(&w.alex, &direct_id, "privat").await;

    let (status, group) = w.add(&w.alex, &direct_id, &[&carl_id]).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    assert_eq!(group["kind"], "group_dm");
    assert_ne!(group["id"], direct["id"]);
    let names: Vec<&str> =
        group["recipients"].as_array().unwrap().iter().map(|u| s(&u["username"])).collect();
    assert_eq!(names, ["alex", "bob", "carl"]);
    assert_eq!(next_of(&mut bob_ws, "DM_CREATE").await["id"], group["id"]);
    assert_eq!(next_of(&mut carl_ws, "DM_CREATE").await["id"], group["id"]);

    // Die 1:1-DM bleibt samt Verlauf; der neue Teilnehmer sieht ihn nicht.
    let (_, dms) = w.app.call("GET", "/dms", Some(&w.alex), None).await;
    assert_eq!(find(&dms, &direct["id"])["recipients"].as_array().unwrap().len(), 2);
    let (status, _) = w.app.call("GET", &format!("/channels/{direct_id}/messages"), Some(&carl), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) =
        w.app.call("GET", &format!("/channels/{}/messages", s(&group["id"])), Some(&carl), None).await;
    assert_eq!(list, json!([]));

    // Wer schon drin ist, sich selbst, leere Liste: ungültig.
    for ids in [vec![w.bob_id.as_str()], vec![w.alex_id.as_str()], vec![]] {
        let (status, _) = w.add(&w.alex, &direct_id, &ids).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    // Nicht erreichbar (keine gemeinsame Community) oder unbekannt.
    let (status, body) = w.add(&w.alex, &direct_id, &[&w.eve_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    // Nicht-Teilnehmer: 404.
    let (status, _) = w.add(&carl, &direct_id, &[&w.bob_id]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Blockade mit dem Neuen oder mit dem Gegenüber: keine Gruppe.
    w.block(&carl, &w.alex_id).await;
    let (status, body) = w.add(&w.alex, &direct_id, &[&carl_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    let dave = w.app.invited_user(&w.alex, &w.community, "dave").await;
    let dave_id = w.app.user_id(&dave).await;
    w.block(&w.alex, &w.bob_id).await;
    let (status, body) = w.add(&w.alex, &direct_id, &[&dave_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn adding_to_a_group_extends_it(db: PgPool) {
    let w = world(db).await;
    let carl = w.app.invited_user(&w.alex, &w.community, "carl").await;
    let carl_id = w.app.user_id(&carl).await;
    let dave = w.app.invited_user(&w.alex, &w.community, "dave").await;
    let addr = serve(&w.app).await;
    let (mut bob_ws, _) = connect(addr, &w.bob).await;
    let (mut carl_ws, _) = connect(addr, &carl).await;

    let (_, group) = w.group(&w.alex, &[&w.bob_id], Some("Runde")).await;
    let channel = s(&group["id"]).to_owned();
    next_of(&mut bob_ws, "DM_CREATE").await;
    let before = w.send(&w.alex, &channel, "vorher").await;
    let (_, _) = w.app.call("PUT", &format!("/dms/{channel}/pin"), Some(&w.bob), None).await;

    // Bob (nicht der Ersteller) fügt Carl hinzu.
    let (status, updated) = w.add(&w.bob, &channel, &[&carl_id, &carl_id]).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["id"], group["id"]);
    assert_eq!(updated["recipients"].as_array().unwrap().len(), 3);
    // Eigene Sicht in der Antwort (Bobs Pin), geteilte Events ohne.
    assert!(updated["pinned_at"].is_string());

    // Carl bekommt die Gruppe, die anderen ein Update; alle die Systemnachricht.
    let created = next_of(&mut carl_ws, "DM_CREATE").await;
    assert_eq!((&created["id"], &created["pinned_at"]), (&group["id"], &Value::Null));
    assert_eq!(created["name"], "Runde");
    let update = next_of(&mut bob_ws, "DM_UPDATE").await;
    assert_eq!(update["recipients"].as_array().unwrap().len(), 3);
    assert_eq!(update["pinned_at"], Value::Null);
    let sys = next_of(&mut carl_ws, "MESSAGE_CREATE").await;
    assert_eq!((s(&sys["kind"]), s(&sys["content"])), ("recipient_add", ""));
    assert_eq!(sys["author"]["id"], w.bob_id.as_str());
    assert_eq!(sys["mentions"], json!([carl_id]));
    assert_eq!(next_of(&mut bob_ws, "MESSAGE_CREATE").await["id"], sys["id"]);
    assert_none_of(&mut carl_ws, &["DM_UPDATE"], Duration::from_millis(300)).await;

    // Carl sieht den Verlauf und kann schreiben; Systemnachrichten lassen sich nicht bearbeiten.
    let (_, list) = w.app.call("GET", &format!("/channels/{channel}/messages"), Some(&carl), None).await;
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|m| s(&m["id"])).collect();
    assert!(ids.contains(&s(&before["id"])));
    w.send(&carl, &channel, "hallo zusammen").await;
    let (status, _) = w
        .app
        .call("PATCH", &format!("/messages/{}", s(&sys["id"])), Some(&w.bob), Some(json!({ "content": "x" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Doppelt hinzufügen geht nicht; Nicht-Teilnehmer bekommen 404; Unerreichbare 403.
    let (status, _) = w.add(&w.alex, &channel, &[&carl_id]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = w.add(&dave, &channel, &[&w.eve_id]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = w.add(&w.alex, &channel, &[&w.eve_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));
    // Blockade zwischen Hinzufügendem und Neuem.
    let dave_id = w.app.user_id(&dave).await;
    w.block(&dave, &w.alex_id).await;
    let (status, body) = w.add(&w.alex, &channel, &[&dave_id]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "dm_not_allowed"));

    // Limit: 3 + 7 = 10 geht, eine Person mehr nicht.
    let mut ids = Vec::new();
    for i in 0..8 {
        let t = w.app.invited_user(&w.alex, &w.community, &format!("user{i}")).await;
        ids.push(w.app.user_id(&t).await);
    }
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (status, body) = w.add(&w.alex, &channel, &refs).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_recipients"));
    let (status, full) = w.add(&w.alex, &channel, &refs[..7]).await;
    assert_eq!(status, StatusCode::OK, "{full}");
    assert_eq!(full["recipients"].as_array().unwrap().len(), 10);
    let (status, body) = w.add(&w.alex, &channel, &refs[7..]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_recipients"));
}
