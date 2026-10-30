//! Integrationstests für Paket D: Präsenz, Tipp-Anzeige und Lesestände.
//! Präsenz und Tippen laufen über echte WebSocket-Verbindungen gegen einen lokal gestarteten Server.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::StatusCode;
use common::{TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Startet einen echten Server für WebSocket-Verbindungen.
async fn serve(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

/// Verbindet, identifiziert sich und gibt Verbindung + READY-Daten zurück.
async fn connect(addr: SocketAddr, token: &str, query: &str) -> (Ws, Value) {
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway{query}")).await.unwrap();
    op(&mut ws, json!({ "op": "identify", "token": token })).await;
    let ready = next(&mut ws).await;
    assert_eq!(ready["t"], "READY");
    (ws, ready["d"].clone())
}

async fn op(ws: &mut Ws, value: Value) {
    ws.send(tungstenite::Message::Text(value.to_string().into())).await.unwrap();
}

/// Nächstes Text-Event (max. 5 s).
async fn next(ws: &mut Ws) -> Value {
    try_next(ws, Duration::from_secs(5)).await.expect("kein Event binnen 5 s")
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

/// Nächstes Event eines bestimmten Typs; andere werden übersprungen.
async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let event = next(ws).await;
        if event["t"] == kind {
            return event["d"].clone();
        }
    }
}

/// Stellt sicher, dass binnen `wait` kein Event des Typs kommt.
async fn assert_none_of(ws: &mut Ws, kind: &str, wait: Duration) {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return;
        }
        if let Some(event) = try_next(ws, left).await {
            assert_ne!(event["t"], kind, "unerwartetes Event: {event}");
        }
    }
}

/// Status laut READY/RESYNC: Offline-Nutzer fehlen dort (fehlend = offline).
fn presence_of(ready: &Value, user_id: &str) -> Value {
    let presences = ready["presences"].as_array().unwrap();
    assert!(presences.iter().all(|p| p["status"] != "offline"), "offline in READY: {ready}");
    presences.iter().find(|p| p["user_id"] == user_id).map_or(json!("offline"), |p| p["status"].clone())
}

/// alex (Besitzer) und bob (Mitglied) in einer Community.
async fn setup(app: &TestApp) -> (String, String, TestCommunity) {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    (alex, bob, community)
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn presence_follows_connections(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _) = setup(&app).await;
    let alex_id = app.user_id(&alex).await;
    let bob_id = app.user_id(&bob).await;
    let addr = serve(&app).await;

    let (mut alex_ws, ready) = connect(addr, &alex, "").await;
    assert_eq!(presence_of(&ready, &bob_id), "offline");
    assert_eq!(presence_of(&ready, &alex_id), "online");

    // Bob kommt online → Alex erfährt es; Bobs READY kennt Alex als online.
    let (mut bob_ws, ready) = connect(addr, &bob, "").await;
    assert_eq!(presence_of(&ready, &alex_id), "online");
    let update = next_of(&mut alex_ws, "PRESENCE_UPDATE").await;
    assert_eq!(update, json!({ "user_id": bob_id, "status": "online" }));

    // Zweite Verbindung von Bob ändert nichts; Status-Wechsel schon.
    let (mut bob_ws2, _) = connect(addr, &bob, "").await;
    op(&mut bob_ws, json!({ "op": "presence", "status": "dnd" })).await;
    let update = next_of(&mut alex_ws, "PRESENCE_UPDATE").await;
    assert_eq!(update["status"], "dnd");

    // Die eigene Präsenz geht nicht an die eigenen Verbindungen.
    assert_none_of(&mut bob_ws2, "PRESENCE_UPDATE", Duration::from_millis(300)).await;

    // Erste Verbindung (mit dnd) zu → wieder online; letzte zu → offline.
    bob_ws.close(None).await.unwrap();
    let update = next_of(&mut alex_ws, "PRESENCE_UPDATE").await;
    assert_eq!(update["status"], "online");
    bob_ws2.close(None).await.unwrap();
    let update = next_of(&mut alex_ws, "PRESENCE_UPDATE").await;
    assert_eq!(update, json!({ "user_id": bob_id, "status": "offline" }));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn invisible_users_appear_offline(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    let addr = serve(&app).await;

    let (mut alex_ws, _) = connect(addr, &alex, "").await;

    // Unsichtbar von Anfang an: kein Online-Event, im READY anderer offline.
    let (mut bob_ws, _) = connect(addr, &bob, "?status=invisible").await;
    assert_none_of(&mut alex_ws, "PRESENCE_UPDATE", Duration::from_millis(500)).await;
    let (_alex_ws2, ready) = connect(addr, &alex, "").await;
    assert_eq!(presence_of(&ready, &bob_id), "offline");

    // Sichtbar werden → online, wieder unsichtbar → offline.
    op(&mut bob_ws, json!({ "op": "presence", "status": "online" })).await;
    assert_eq!(next_of(&mut alex_ws, "PRESENCE_UPDATE").await["status"], "online");
    op(&mut bob_ws, json!({ "op": "presence", "status": "invisible" })).await;
    assert_eq!(next_of(&mut alex_ws, "PRESENCE_UPDATE").await["status"], "offline");

    // Abmelden eines Unsichtbaren erzeugt kein Event.
    bob_ws.close(None).await.unwrap();
    assert_none_of(&mut alex_ws, "PRESENCE_UPDATE", Duration::from_millis(500)).await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn typing_respects_visibility_and_throttle(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let alex_id = app.user_id(&alex).await;

    // Privater Kanal, den Bob nicht sieht.
    let (_, private) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "intern", "kind": "text" })),
        )
        .await;
    let private_id = s(&private["id"]).to_owned();
    app.call(
        "PUT",
        &format!("/channels/{private_id}/overrides/{}", community.default_role),
        Some(&alex),
        Some(json!({ "allow": 0, "deny": 1 })),
    )
    .await;

    let addr = serve(&app).await;
    let (mut alex_ws, _) = connect(addr, &alex, "").await;
    let (mut bob_ws, _) = connect(addr, &bob, "").await;

    // Tippen im privaten Kanal erreicht Bob nicht, im öffentlichen schon.
    op(&mut alex_ws, json!({ "op": "typing", "channel_id": private_id })).await;
    op(&mut alex_ws, json!({ "op": "typing", "channel_id": community.text_channel })).await;
    let typing = next_of(&mut bob_ws, "TYPING_START").await;
    assert_eq!(
        typing,
        json!({ "channel_id": community.text_channel, "community_id": community.id, "user_id": alex_id })
    );
    // Alex bekommt seine eigenen Events (beide Kanäle sieht er) auch; der Client blendet sich selbst aus.
    assert_eq!(next_of(&mut alex_ws, "TYPING_START").await["channel_id"], private_id.as_str());
    assert_eq!(next_of(&mut alex_ws, "TYPING_START").await["channel_id"], community.text_channel.as_str());

    // Gedrosselt: ein zweites Tippen binnen 3 s erzeugt kein Event.
    op(&mut alex_ws, json!({ "op": "typing", "channel_id": community.text_channel })).await;
    assert_none_of(&mut bob_ws, "TYPING_START", Duration::from_millis(500)).await;

    // Bob tippt im privaten Kanal (unsichtbar für ihn) bzw. im Voice-Kanal → nichts.
    op(&mut bob_ws, json!({ "op": "typing", "channel_id": private_id })).await;
    op(&mut bob_ws, json!({ "op": "typing", "channel_id": community.voice_channel })).await;
    assert_none_of(&mut alex_ws, "TYPING_START", Duration::from_millis(500)).await;

    // Ohne SEND_MESSAGES (Override auf dem Kanal) wird Tippen ignoriert.
    app.call(
        "PUT",
        &format!("/channels/{}/overrides/{}", community.text_channel, community.default_role),
        Some(&alex),
        Some(json!({ "allow": 0, "deny": 2 })),
    )
    .await;
    op(&mut bob_ws, json!({ "op": "typing", "channel_id": community.text_channel })).await;
    assert_none_of(&mut alex_ws, "TYPING_START", Duration::from_millis(500)).await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn read_states_move_forward_and_sync(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let channel = &community.text_channel;

    let mut ids = Vec::new();
    for content in ["eins", "zwei", "drei"] {
        let (status, msg) = app
            .call(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(&alex),
                Some(json!({ "content": content })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        ids.push(s(&msg["id"]).to_owned());
    }

    // Noch nichts gelesen: kein Lesestand, aber die neueste Nachricht ist bekannt.
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    assert!(detail["read_states"].get(channel).is_none());
    assert_eq!(detail["last_message_ids"][channel], ids[2].as_str());
    // Voice-Kanäle haben keinen Lesestand.
    assert!(detail["last_message_ids"].get(&community.voice_channel).is_none());

    let addr = serve(&app).await;
    let (mut bob_ws, _) = connect(addr, &bob, "").await;

    let read = |id: &str| json!({ "message_id": id });
    let (status, _) =
        app.call("PUT", &format!("/channels/{channel}/read"), Some(&bob), Some(read(&ids[1]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let update = next_of(&mut bob_ws, "READ_STATE_UPDATE").await;
    assert_eq!(
        update,
        json!({ "channel_id": channel, "community_id": community.id, "last_read_message_id": ids[1] })
    );

    // Rückwärts setzen ist wirkungslos (und erzeugt kein Event).
    let (status, _) =
        app.call("PUT", &format!("/channels/{channel}/read"), Some(&bob), Some(read(&ids[0]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_none_of(&mut bob_ws, "READ_STATE_UPDATE", Duration::from_millis(300)).await;
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    assert_eq!(detail["read_states"][channel], ids[1].as_str());

    // Lesestände sind pro Nutzer.
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert!(detail["read_states"].get(channel).is_none());

    // Fremde/unbekannte Nachricht → 404; Voice-Kanal → 400.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/channels/{channel}/read"),
            Some(&bob),
            Some(read("01890000-0000-7000-8000-000000000000")),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call("PUT", &format!("/channels/{}/read", community.voice_channel), Some(&bob), Some(read(&ids[2])))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Privater Kanal: für Bob unsichtbar → 404, und nicht im Detail.
    let (_, private) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "intern", "kind": "text" })),
        )
        .await;
    let private_id = s(&private["id"]).to_owned();
    app.call(
        "PUT",
        &format!("/channels/{private_id}/overrides/{}", community.default_role),
        Some(&alex),
        Some(json!({ "allow": 0, "deny": 1 })),
    )
    .await;
    let (_, secret) = app
        .call(
            "POST",
            &format!("/channels/{private_id}/messages"),
            Some(&alex),
            Some(json!({ "content": "geheim" })),
        )
        .await;
    let (status, _) = app
        .call("PUT", &format!("/channels/{private_id}/read"), Some(&bob), Some(read(s(&secret["id"]))))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    assert!(detail["last_message_ids"].get(&private_id).is_none());
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert_eq!(detail["last_message_ids"][&private_id], s(&secret["id"]));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn user_update_and_account_deletion_events(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    // Carol teilt keine Community mit Bob.
    let other = app.community(&alex, "Andere").await;
    let carol = app.invited_user(&alex, &other, "carol").await;
    let addr = serve(&app).await;

    let (mut alex_ws, _) = connect(addr, &alex, "").await;
    let (mut bob_ws, _) = connect(addr, &bob, "").await;
    let (mut carol_ws, _) = connect(addr, &carol, "").await;

    let (status, _) = app
        .call("PATCH", "/auth/me", Some(&bob), Some(json!({ "bio": "Neu hier", "status_text": "Kaffee" })))
        .await;
    assert_eq!(status, StatusCode::OK);
    let update = next_of(&mut alex_ws, "USER_UPDATE").await;
    assert_eq!(update["id"], bob_id.as_str());
    assert_eq!(update["bio"], "Neu hier");
    assert_eq!(update["status_text"], "Kaffee");
    assert_eq!(next_of(&mut bob_ws, "USER_UPDATE").await["bio"], "Neu hier");
    assert_none_of(&mut carol_ws, "USER_UPDATE", Duration::from_millis(300)).await;

    // Konto löschen: Mitglieder erfahren es, Bobs Verbindung wird geschlossen.
    let (status, _) =
        app.call("DELETE", "/auth/me", Some(&bob), Some(json!({ "password": common::PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let removed = next_of(&mut alex_ws, "MEMBER_REMOVE").await;
    assert_eq!(removed, json!({ "community_id": community.id, "user_id": bob_id }));
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match bob_ws.next().await {
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

/// Beitritt: Status der neuen Mitglieder kommen als ein `PRESENCE_BULK` – ohne Offline-Nutzer und
/// ohne bereits bekannte Status.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn join_sends_presence_bulk(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _) = setup(&app).await;
    let other = app.community(&alex, "Andere").await;
    let carol = app.invited_user(&alex, &other, "carol").await;
    app.invited_user(&alex, &other, "dave").await;
    let carol_id = app.user_id(&carol).await;
    let addr = serve(&app).await;

    let (_alex_ws, _) = connect(addr, &alex, "").await;
    let (_carol_ws, _) = connect(addr, &carol, "?status=dnd").await;
    let (mut bob_ws, ready) = connect(addr, &bob, "").await;
    // Nur alex und Bob selbst; carol ist in keiner gemeinsamen Community.
    assert_eq!(ready["presences"].as_array().unwrap().len(), 2, "{ready}");

    let (_, invite) =
        app.call("POST", &format!("/communities/{}/invites", other.id), Some(&alex), Some(json!({}))).await;
    let (status, _) =
        app.call("POST", &format!("/invites/{}/accept", s(&invite["code"])), Some(&bob), None).await;
    assert!(status.is_success(), "{status}");

    // alex kennt Bob schon, dave ist offline → nur carol.
    let bulk = next_of(&mut bob_ws, "PRESENCE_BULK").await;
    assert_eq!(bulk, json!({ "presences": [{ "user_id": carol_id, "status": "dnd" }] }));
    assert_none_of(&mut bob_ws, "PRESENCE_UPDATE", Duration::from_millis(300)).await;
}

/// `USER_UPDATE` kommt bei mehreren gemeinsamen Communities und einer DM nur einmal an.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn user_update_arrives_once(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _) = setup(&app).await;
    let second = app.community(&alex, "Zweite").await;
    let (_, invite) =
        app.call("POST", &format!("/communities/{}/invites", second.id), Some(&alex), Some(json!({}))).await;
    app.call("POST", &format!("/invites/{}/accept", s(&invite["code"])), Some(&bob), None).await;
    let alex_id = app.user_id(&alex).await;
    let (status, _) = app.call("POST", "/dms", Some(&bob), Some(json!({ "recipient_ids": [alex_id] }))).await;
    assert!(status.is_success(), "{status}");
    let addr = serve(&app).await;
    let (mut alex_ws, _) = connect(addr, &alex, "").await;
    let (mut bob_ws, _) = connect(addr, &bob, "").await;

    let (status, _) = app.call("PATCH", "/auth/me", Some(&bob), Some(json!({ "bio": "Einmal" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(next_of(&mut alex_ws, "USER_UPDATE").await["bio"], "Einmal");
    assert_none_of(&mut alex_ws, "USER_UPDATE", Duration::from_millis(300)).await;
    assert_eq!(next_of(&mut bob_ws, "USER_UPDATE").await["bio"], "Einmal");
    assert_none_of(&mut bob_ws, "USER_UPDATE", Duration::from_millis(300)).await;
}
