//! Integrationstests für Voice (LiveKit-Tokens, Webhooks, Voice-States, Rechteentzug).
//! Eine echte LiveKit-Instanz ist nicht nötig: Webhooks werden hier selbst signiert,
//! die RoomService-API wird durch einen kleinen Mock-Server ersetzt.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode, header},
    routing::post,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use common::{TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use lykos_server::config::{Config, LiveKitConfig};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;

const VIEW_CHANNEL: i64 = 1 << 0;
const CONNECT: i64 = 1 << 4;
const SPEAK: i64 = 1 << 5;
const STREAM: i64 = 1 << 6;
const VIDEO: i64 = 1 << 13;

fn voice_app(db: PgPool) -> TestApp {
    TestApp::with_config(db, Config::for_tests_with_voice())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

/// HS256-JWT, unabhängig von der Server-Implementierung gebaut.
fn jwt(claims: &Value, secret: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let input = format!("{header}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Payload eines JWT (ohne Prüfung).
fn jwt_claims(token: &str) -> Value {
    let payload = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

/// Webhook so signieren, wie LiveKit es tut: JWT mit `sha256` = Base64(SHA-256(Body)).
fn webhook_auth(body: &str, key: &str, secret: &str) -> String {
    let hash = STANDARD.encode(Sha256::digest(body.as_bytes()));
    jwt(&json!({ "iss": key, "nbf": now(), "exp": now() + 300, "sha256": hash }), secret)
}

async fn send_webhook(app: &TestApp, body: &Value, auth: Option<String>) -> StatusCode {
    let body = body.to_string();
    let cfg = LiveKitConfig::for_tests();
    let auth = auth.unwrap_or_else(|| webhook_auth(&body, &cfg.api_key, &cfg.api_secret));
    let req = Request::builder()
        .method("POST")
        .uri("/api/livekit/webhook")
        .header(header::CONTENT_TYPE, "application/webhook+json")
        .header(header::AUTHORIZATION, auth)
        .body(Body::from(body))
        .unwrap();
    app.send(req).await.0
}

fn participant_event(event: &str, room: &str, identity: &str, sid: &str) -> Value {
    json!({
        "event": event,
        "room": { "sid": "RM_test", "name": room },
        "participant": { "sid": sid, "identity": identity, "name": "x", "state": "ACTIVE" },
        "id": "EV_test",
        "createdAt": now().to_string(),
    })
}

fn track_event(event: &str, room: &str, identity: &str, sid: &str, source: &str) -> Value {
    let mut ev = participant_event(event, room, identity, sid);
    ev["track"] = json!({ "sid": "TR_test", "type": "VIDEO", "source": source });
    ev
}

async fn voice_states(app: &TestApp, token: &str, community: &TestCommunity) -> Vec<Value> {
    let (status, list) =
        app.call("GET", &format!("/communities/{}/voice-states", community.id), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list.as_array().unwrap().clone()
}

async fn set_override(app: &TestApp, owner: &str, channel: &str, role: &str, allow: i64, deny: i64) {
    let (status, body) = app
        .call(
            "PUT",
            &format!("/channels/{channel}/overrides/{role}"),
            Some(owner),
            Some(json!({ "allow": allow, "deny": deny })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

// --- Gateway-Helfer ----------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_gateway(app: &TestApp, token: &str) -> Ws {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    ws.send(tungstenite::Message::Text(json!({ "op": "identify", "token": token }).to_string().into()))
        .await
        .unwrap();
    assert_eq!(next(&mut ws).await["t"], "READY");
    ws
}

/// Nächstes Text-Event vom Gateway (max. 5 s).
async fn next(ws: &mut Ws) -> Value {
    let fut = async {
        loop {
            if let tungstenite::Message::Text(t) = ws.next().await.unwrap().unwrap() {
                return serde_json::from_str::<Value>(&t).unwrap();
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut).await.expect("kein Event binnen 5 s")
}

// --- LiveKit-Mock --------------------------------------------------------------

/// Aufgezeichneter RoomService-Aufruf: (Methode, Authorization-Header, Body).
type Call = (String, String, Value);

/// Antworten des Mocks je Methode (sonst `{}`); im Test nachträglich befüllbar.
type MockResponses = Arc<Mutex<HashMap<String, Value>>>;

async fn mock_livekit() -> (String, mpsc::UnboundedReceiver<Call>) {
    let (url, rx, _) = mock_livekit_with_responses().await;
    (url, rx)
}

async fn mock_livekit_with_responses() -> (String, mpsc::UnboundedReceiver<Call>, MockResponses) {
    type MockState = (mpsc::UnboundedSender<Call>, MockResponses);
    async fn handle(
        State((tx, responses)): State<MockState>,
        Path(method): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        let response = responses.lock().unwrap().get(&method).cloned().unwrap_or_else(|| json!({}));
        let _ = tx.send((method, auth, body));
        Json(response)
    }
    let responses = MockResponses::default();
    let (tx, rx) = mpsc::unbounded_channel();
    let router = Router::new()
        .route("/twirp/livekit.RoomService/{method}", post(handle))
        .with_state((tx, responses.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), rx, responses)
}

async fn next_call(rx: &mut mpsc::UnboundedReceiver<Call>) -> Call {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("kein RoomService-Aufruf binnen 5 s")
        .unwrap()
}

/// Wartet, bis die Voice-States die Bedingung erfüllen (Rechteprüfung läuft im Hintergrund).
async fn wait_for_states(
    app: &TestApp,
    token: &str,
    community: &TestCommunity,
    cond: impl Fn(&[Value]) -> bool,
) {
    for _ in 0..50 {
        if cond(&voice_states(app, token, community).await) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Voice-States erreichen den erwarteten Zustand nicht");
}

// --- Tests -------------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn voice_unavailable_without_livekit(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;

    let (_, instance) = app.call("GET", "/instance", None, None).await;
    assert_eq!(instance["voice"], false);
    let (status, body) = app
        .call("POST", &format!("/channels/{}/voice/join", community.voice_channel), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "voice_unavailable");

    let voice = voice_app(db);
    let (_, instance) = voice.call("GET", "/instance", None, None).await;
    assert_eq!(instance["voice"], true);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn join_token_follows_permissions(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let join = format!("/channels/{}/voice/join", community.voice_channel);

    // Standardrechte: hören, sprechen, Bildschirm teilen, Kamera.
    let (status, body) = app.call("POST", &join, Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "ws://livekit.test");
    let claims = jwt_claims(s(&body["token"]));
    assert_eq!(claims["iss"], "test-key");
    assert_eq!(claims["sub"], bob_id.as_str());
    assert_eq!(claims["name"], "bob");
    assert!(claims["exp"].as_i64().unwrap() - claims["nbf"].as_i64().unwrap() <= 600);
    let video = &claims["video"];
    assert_eq!(video["room"], community.voice_channel.as_str());
    assert_eq!(video["roomJoin"], true);
    assert_eq!(video["canSubscribe"], true);
    assert_eq!(video["canPublish"], true);
    assert_eq!(video["canPublishData"], false);
    assert_eq!(
        video["canPublishSources"],
        json!(["microphone", "screen_share", "screen_share_audio", "camera"])
    );
    assert!(video.get("roomAdmin").is_none());

    // Textkanal: 400.
    let (status, body) =
        app.call("POST", &format!("/channels/{}/voice/join", community.text_channel), Some(&bob), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "not_a_voice_channel");

    // Ohne SPEAK: kein Mikrofon, Bildschirm und Kamera weiterhin.
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, SPEAK).await;
    let (_, body) = app.call("POST", &join, Some(&bob), None).await;
    let video = jwt_claims(s(&body["token"]))["video"].clone();
    assert_eq!(video["canPublish"], true);
    assert_eq!(video["canPublishSources"], json!(["screen_share", "screen_share_audio", "camera"]));

    // Ohne VIDEO: keine Kamera.
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, VIDEO).await;
    let (_, body) = app.call("POST", &join, Some(&bob), None).await;
    let video = jwt_claims(s(&body["token"]))["video"].clone();
    assert_eq!(video["canPublishSources"], json!(["microphone", "screen_share", "screen_share_audio"]));

    // Nur VIDEO: Kamera reicht für canPublish.
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, SPEAK | STREAM).await;
    let (_, body) = app.call("POST", &join, Some(&bob), None).await;
    let video = jwt_claims(s(&body["token"]))["video"].clone();
    assert_eq!(video["canPublish"], true);
    assert_eq!(video["canPublishSources"], json!(["camera"]));

    // Rolle ohne VIDEO, aber im Kanal per Override erlaubt: Kamera geht.
    let (status, body) = app
        .call(
            "PATCH",
            &format!("/roles/{}", community.default_role),
            Some(&alex),
            Some(json!({ "permissions": VIEW_CHANNEL | CONNECT })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    set_override(&app, &alex, &community.voice_channel, &community.default_role, VIDEO, 0).await;
    let (_, body) = app.call("POST", &join, Some(&bob), None).await;
    let video = jwt_claims(s(&body["token"]))["video"].clone();
    assert_eq!(video["canPublishSources"], json!(["camera"]));

    // Ohne SPEAK, STREAM und VIDEO: nur zuhören.
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, SPEAK | STREAM | VIDEO)
        .await;
    let (_, body) = app.call("POST", &join, Some(&bob), None).await;
    let video = jwt_claims(s(&body["token"]))["video"].clone();
    assert_eq!(video["canPublish"], false);
    assert!(video.get("canPublishSources").is_none());

    // Ohne CONNECT: 403.
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, CONNECT).await;
    let (status, body) = app.call("POST", &join, Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "forbidden");
    // Der Besitzer darf trotzdem.
    assert_eq!(app.call("POST", &join, Some(&alex), None).await.0, StatusCode::OK);

    // Unsichtbarer Kanal: 404 (Existenz wird nicht verraten).
    set_override(&app, &alex, &community.voice_channel, &community.default_role, 0, VIEW_CHANNEL).await;
    assert_eq!(app.call("POST", &join, Some(&bob), None).await.0, StatusCode::NOT_FOUND);

    // Ohne Anmeldung: 401.
    assert_eq!(app.call("POST", &join, None, None).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn webhook_signature_is_verified(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let alex_id = app.user_id(&alex).await;
    let community = app.community(&alex, "Rudel").await;
    let ev = participant_event("participant_joined", &community.voice_channel, &alex_id, "PA_1");
    let body = ev.to_string();
    let cfg = LiveKitConfig::for_tests();

    // Falsches Secret, falscher Key, manipulierter Body, fehlender Header: abgelehnt.
    let wrong_secret = webhook_auth(&body, &cfg.api_key, "falsches-secret");
    assert_eq!(send_webhook(&app, &ev, Some(wrong_secret)).await, StatusCode::UNAUTHORIZED);
    let wrong_key = webhook_auth(&body, "anderer-key", &cfg.api_secret);
    assert_eq!(send_webhook(&app, &ev, Some(wrong_key)).await, StatusCode::UNAUTHORIZED);
    let other_body = webhook_auth("{}", &cfg.api_key, &cfg.api_secret);
    assert_eq!(send_webhook(&app, &ev, Some(other_body)).await, StatusCode::UNAUTHORIZED);
    let expired = jwt(
        &json!({ "iss": cfg.api_key, "nbf": now() - 7200, "exp": now() - 3600,
                 "sha256": STANDARD.encode(Sha256::digest(body.as_bytes())) }),
        &cfg.api_secret,
    );
    assert_eq!(send_webhook(&app, &ev, Some(expired)).await, StatusCode::UNAUTHORIZED);
    let req = Request::builder().method("POST").uri("/api/livekit/webhook").body(Body::from(body)).unwrap();
    assert_eq!(app.send(req).await.0, StatusCode::UNAUTHORIZED);
    assert!(voice_states(&app, &alex, &community).await.is_empty());

    // Korrekt signiert: angenommen und verarbeitet.
    assert_eq!(send_webhook(&app, &ev, None).await, StatusCode::OK);
    let states = voice_states(&app, &alex, &community).await;
    assert_eq!(states.len(), 1);
    assert_eq!(states[0]["user_id"], alex_id.as_str());

    // Ohne LiveKit-Konfiguration gibt es keinen Webhook-Empfang.
    let plain = TestApp::new(app.db.clone());
    assert_eq!(send_webhook(&plain, &ev, None).await, StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn voice_state_events(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let room = community.voice_channel.clone();

    let mut ws = connect_gateway(&app, &alex).await;

    // Beitritt.
    let joined = participant_event("participant_joined", &room, &bob_id, "PA_1");
    assert_eq!(send_webhook(&app, &joined, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["community_id"], community.id.as_str());
    assert_eq!(ev["d"]["channel_id"], room.as_str());
    assert_eq!(ev["d"]["user_id"], bob_id.as_str());
    assert_eq!(ev["d"]["self_mute"], false);
    assert_eq!(ev["d"]["self_deaf"], false);
    assert_eq!(ev["d"]["streaming"], false);

    // Mute/Deaf meldet der Client selbst.
    let (status, _) = app
        .call("POST", "/voice/state", Some(&bob), Some(json!({ "self_mute": true, "self_deaf": true })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["self_mute"], true);
    assert_eq!(ev["d"]["self_deaf"], true);

    // Bildschirm teilen: Mikrofon-Tracks ändern nichts, Screenshare schon.
    let mic = track_event("track_published", &room, &bob_id, "PA_1", "MICROPHONE");
    assert_eq!(send_webhook(&app, &mic, None).await, StatusCode::OK);
    let screen = track_event("track_published", &room, &bob_id, "PA_1", "SCREEN_SHARE");
    assert_eq!(send_webhook(&app, &screen, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["streaming"], true);
    assert_eq!(ev["d"]["self_mute"], true, "Mute bleibt erhalten");

    let states = voice_states(&app, &alex, &community).await;
    assert_eq!(states.len(), 1);
    assert_eq!(states[0]["streaming"], true);

    let unscreen = track_event("track_unpublished", &room, &bob_id, "PA_1", "SCREEN_SHARE");
    assert_eq!(send_webhook(&app, &unscreen, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["d"]["streaming"], false);

    // Neue Verbindung (z. B. zweites Gerät) – das verspätete `left` der alten wird ignoriert.
    let rejoined = participant_event("participant_joined", &room, &bob_id, "PA_2");
    assert_eq!(send_webhook(&app, &rejoined, None).await, StatusCode::OK);
    assert_eq!(next(&mut ws).await["t"], "VOICE_STATE_UPDATE");
    let stale_left = participant_event("participant_left", &room, &bob_id, "PA_1");
    assert_eq!(send_webhook(&app, &stale_left, None).await, StatusCode::OK);
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 1);

    // Kanalwechsel: DELETE für den alten, UPDATE für den neuen Kanal.
    let (_, lounge) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    let lounge_id = s(&lounge["id"]).to_owned();
    assert_eq!(next(&mut ws).await["t"], "CHANNEL_CREATE");
    let moved = participant_event("participant_joined", &lounge_id, &bob_id, "PA_3");
    assert_eq!(send_webhook(&app, &moved, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_DELETE");
    assert_eq!(ev["d"], json!({ "community_id": community.id, "channel_id": room, "user_id": bob_id }));
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["channel_id"], lounge_id.as_str());

    // Verlassen.
    let left = participant_event("participant_left", &lounge_id, &bob_id, "PA_3");
    assert_eq!(send_webhook(&app, &left, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_DELETE");
    assert_eq!(ev["d"]["channel_id"], lounge_id.as_str());
    assert!(voice_states(&app, &alex, &community).await.is_empty());

    // Mute ohne Voice-Kanal ist wirkungslos.
    let (status, _) = app
        .call("POST", "/voice/state", Some(&bob), Some(json!({ "self_mute": false, "self_deaf": false })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // room_finished räumt alle Teilnehmer des Raums ab.
    let alex_id = app.user_id(&alex).await;
    send_webhook(&app, &participant_event("participant_joined", &room, &alex_id, "PA_4"), None).await;
    send_webhook(&app, &participant_event("participant_joined", &room, &bob_id, "PA_5"), None).await;
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 2);
    let finished = json!({ "event": "room_finished", "room": { "name": room } });
    assert_eq!(send_webhook(&app, &finished, None).await, StatusCode::OK);
    assert!(voice_states(&app, &alex, &community).await.is_empty());

    // Fremde Räume/Identities werden ignoriert.
    let foreign = participant_event("participant_joined", "kein-kanal", "kein-nutzer", "PA_X");
    assert_eq!(send_webhook(&app, &foreign, None).await, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn hidden_voice_channels_are_filtered(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let alex_id = app.user_id(&alex).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;

    let (_, secret) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "Geheim", "kind": "voice" })),
        )
        .await;
    let secret_id = s(&secret["id"]).to_owned();
    set_override(&app, &alex, &secret_id, &community.default_role, 0, VIEW_CHANNEL).await;

    send_webhook(&app, &participant_event("participant_joined", &secret_id, &alex_id, "PA_1"), None).await;
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 1);
    assert!(voice_states(&app, &bob, &community).await.is_empty());

    // Nichtmitglieder sehen nichts.
    let (_, other) = app
        .call("POST", &format!("/communities/{}/invites", community.id), Some(&alex), Some(json!({})))
        .await;
    let carol = app.user("carol", Some(s(&other["code"]))).await;
    app.call(
        "DELETE",
        &format!("/communities/{}/members/{}", community.id, app.user_id(&carol).await),
        Some(&carol),
        None,
    )
    .await;
    let (status, _) =
        app.call("GET", &format!("/communities/{}/voice-states", community.id), Some(&carol), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn revoked_rights_remove_participants(db: PgPool) {
    let (api_url, mut calls) = mock_livekit().await;
    let mut config = Config::for_tests_with_voice();
    config.livekit.as_mut().unwrap().api_url = api_url;
    let app = TestApp::with_config(db, config);
    let cfg = LiveKitConfig::for_tests();

    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let carol = app.invited_user(&alex, &community, "carol").await;
    let carol_id = app.user_id(&carol).await;
    let room = community.voice_channel.clone();

    // Kick: Voice-State weg, RemoveParticipant mit Admin-Token.
    send_webhook(&app, &participant_event("participant_joined", &room, &bob_id, "PA_1"), None).await;
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 1);
    let mut ws = connect_gateway(&app, &alex).await;
    let (status, _) = app
        .call("DELETE", &format!("/communities/{}/members/{bob_id}", community.id), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(voice_states(&app, &alex, &community).await.is_empty());
    loop {
        let ev = next(&mut ws).await;
        if ev["t"] == "VOICE_STATE_DELETE" {
            assert_eq!(ev["d"]["user_id"], bob_id.as_str());
            break;
        }
    }
    let (method, auth, body) = next_call(&mut calls).await;
    assert_eq!(method, "RemoveParticipant");
    assert_eq!(body, json!({ "room": room, "identity": bob_id }));
    let token = auth.strip_prefix("Bearer ").expect("Bearer-Token");
    let claims = lykos_server::livekit::verify(token, &cfg.api_key, &cfg.api_secret).unwrap();
    let video = claims.video.unwrap();
    assert!(video.room_admin);
    assert_eq!(video.room.as_deref(), Some(room.as_str()));

    // Override entzieht CONNECT: Carol fliegt raus (Prüfung läuft im Hintergrund).
    send_webhook(&app, &participant_event("participant_joined", &room, &carol_id, "PA_2"), None).await;
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 1);
    set_override(&app, &alex, &room, &community.default_role, 0, CONNECT).await;
    wait_for_states(&app, &alex, &community, |s| s.is_empty()).await;
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "RemoveParticipant");
    assert_eq!(body["identity"], carol_id.as_str());

    // Beitritt ohne CONNECT (Token noch gültig): wird nicht übernommen, sondern entfernt.
    send_webhook(&app, &participant_event("participant_joined", &room, &carol_id, "PA_3"), None).await;
    assert!(voice_states(&app, &alex, &community).await.is_empty());
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "RemoveParticipant");
    assert_eq!(body["identity"], carol_id.as_str());

    // Kanal löschen schließt den Raum.
    let (status, _) = app.call("DELETE", &format!("/channels/{room}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "DeleteRoom");
    assert_eq!(body["room"], room.as_str());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn revoked_publish_rights_update_participant(db: PgPool) {
    let (api_url, mut calls, responses) = mock_livekit_with_responses().await;
    let mut config = Config::for_tests_with_voice();
    config.livekit.as_mut().unwrap().api_url = api_url;
    let app = TestApp::with_config(db, config);

    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let room = community.voice_channel.clone();
    let all_sources = json!(["MICROPHONE", "SCREEN_SHARE", "SCREEN_SHARE_AUDIO", "CAMERA"]);

    // Bob ist im Call: Mikrofon, Bildschirm (+Ton) und Kamera laufen.
    send_webhook(&app, &participant_event("participant_joined", &room, &bob_id, "PA_1"), None).await;
    send_webhook(&app, &track_event("track_published", &room, &bob_id, "PA_1", "SCREEN_SHARE"), None).await;
    send_webhook(&app, &track_event("track_published", &room, &bob_id, "PA_1", "CAMERA"), None).await;
    wait_for_states(&app, &alex, &community, |s| s[0]["streaming"] == true && s[0]["video"] == true).await;
    responses.lock().unwrap().insert(
        "GetParticipant".into(),
        json!({
            "sid": "PA_1",
            "identity": bob_id,
            "permission": { "canSubscribe": true, "canPublish": true, "canPublishSources": all_sources },
            "tracks": [
                { "sid": "TR_MIC", "type": "AUDIO", "source": "MICROPHONE" },
                { "sid": "TR_SCREEN", "type": "VIDEO", "source": "SCREEN_SHARE" },
                { "sid": "TR_SCREEN_AUDIO", "type": "AUDIO", "source": "SCREEN_SHARE_AUDIO" },
                { "sid": "TR_CAM", "type": "VIDEO", "source": "CAMERA" },
            ],
        }),
    );

    // Kanal-Override entzieht STREAM: Bob bleibt, darf aber nur noch Mikrofon und Kamera.
    set_override(&app, &alex, &room, &community.default_role, 0, STREAM).await;
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "GetParticipant");
    assert_eq!(body, json!({ "room": room, "identity": bob_id }));
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "UpdateParticipant");
    assert_eq!(body["room"], room.as_str());
    assert_eq!(body["identity"], bob_id.as_str());
    assert_eq!(
        body["permission"],
        json!({
            "canSubscribe": true,
            "canPublish": true,
            "canPublishData": false,
            "canPublishSources": ["MICROPHONE", "CAMERA"],
        })
    );
    for sid in ["TR_SCREEN", "TR_SCREEN_AUDIO"] {
        let (method, _, body) = next_call(&mut calls).await;
        assert_eq!(method, "MutePublishedTrack");
        assert_eq!(body, json!({ "room": room, "identity": bob_id, "trackSid": sid, "muted": true }));
    }
    wait_for_states(&app, &alex, &community, |s| s[0]["streaming"] == false && s[0]["video"] == true).await;
    assert_eq!(voice_states(&app, &alex, &community).await.len(), 1, "Bob bleibt im Kanal");

    // Rollen-Änderung nimmt SPEAK und VIDEO: nur noch zuhören, alle Spuren stumm.
    let (status, body) = app
        .call(
            "PATCH",
            &format!("/roles/{}", community.default_role),
            Some(&alex),
            Some(json!({ "permissions": VIEW_CHANNEL | CONNECT })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(next_call(&mut calls).await.0, "GetParticipant");
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "UpdateParticipant");
    assert_eq!(body["permission"]["canPublish"], false);
    assert_eq!(body["permission"]["canPublishSources"], json!([]));
    let mut muted = Vec::new();
    for _ in 0..4 {
        let (method, _, body) = next_call(&mut calls).await;
        assert_eq!(method, "MutePublishedTrack");
        muted.push(body["trackSid"].as_str().unwrap().to_owned());
    }
    assert_eq!(muted, ["TR_MIC", "TR_SCREEN", "TR_SCREEN_AUDIO", "TR_CAM"]);
    wait_for_states(&app, &alex, &community, |s| s[0]["video"] == false).await;

    // Beitritt mit älterem Token (mehr Rechte als aktuell): sofort angleichen, ohne Nachfrage.
    let carol = app.invited_user(&alex, &community, "carol").await;
    let carol_id = app.user_id(&carol).await;
    let mut joined = participant_event("participant_joined", &room, &carol_id, "PA_2");
    joined["participant"]["permission"] =
        json!({ "canSubscribe": true, "canPublish": true, "canPublishSources": all_sources });
    assert_eq!(send_webhook(&app, &joined, None).await, StatusCode::OK);
    let (method, _, body) = next_call(&mut calls).await;
    assert_eq!(method, "UpdateParticipant");
    assert_eq!(body["identity"], carol_id.as_str());
    assert_eq!(body["permission"]["canPublish"], false);

    // Passende Rechte beim Beitritt: kein RoomService-Aufruf.
    let alex_id = app.user_id(&alex).await;
    let mut joined = participant_event("participant_joined", &room, &alex_id, "PA_3");
    joined["participant"]["permission"] =
        json!({ "canSubscribe": true, "canPublish": true, "canPublishSources": all_sources });
    assert_eq!(send_webhook(&app, &joined, None).await, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(calls.try_recv().is_err(), "Besitzer hat alle Rechte – nichts anzugleichen");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn video_migration_extends_default_roles(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let (status, mods) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(&alex),
            Some(json!({ "name": "Mods", "permissions": 1 << 9 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{mods}");
    let role_perms = |id: String| {
        let db = db.clone();
        async move {
            let id: uuid::Uuid = id.parse().unwrap();
            sqlx::query_scalar::<_, i64>("SELECT permissions FROM roles WHERE id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };

    // Neue Communities bekommen VIDEO direkt.
    assert_eq!(role_perms(community.default_role.clone()).await & VIDEO, VIDEO);

    // Stand vor der Migration nachbauen und 0007 erneut ausführen.
    sqlx::query("UPDATE roles SET permissions = permissions & ~$1::BIGINT")
        .bind(VIDEO)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(role_perms(community.default_role.clone()).await & VIDEO, 0);
    assert!(lykos_server::MIGRATOR.iter().any(|m| m.version == 7), "Migration 0007 ist registriert");
    sqlx::raw_sql(include_str!("../migrations/0007_video_permission.sql")).execute(&db).await.unwrap();

    let perms = role_perms(community.default_role.clone()).await;
    assert_eq!(perms & VIDEO, VIDEO, "Standardrolle bekommt VIDEO");
    let voice = VIEW_CHANNEL | CONNECT | SPEAK | STREAM;
    assert_eq!(perms & voice, voice, "übrige Rechte bleiben");
    assert_eq!(role_perms(s(&mods["id"]).to_owned()).await, 1 << 9, "andere Rollen bleiben unverändert");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn camera_sets_video_flag(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let room = community.voice_channel.clone();

    let mut ws = connect_gateway(&app, &alex).await;
    send_webhook(&app, &participant_event("participant_joined", &room, &bob_id, "PA_1"), None).await;
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["video"], false);

    // Kamera an: video = true, streaming bleibt aus.
    let cam = track_event("track_published", &room, &bob_id, "PA_1", "CAMERA");
    assert_eq!(send_webhook(&app, &cam, None).await, StatusCode::OK);
    let ev = next(&mut ws).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["video"], true);
    assert_eq!(ev["d"]["streaming"], false);
    assert_eq!(voice_states(&app, &alex, &community).await[0]["video"], true);

    // Bildschirm dazu: beide Flags sind unabhängig.
    let screen = track_event("track_published", &room, &bob_id, "PA_1", "SCREEN_SHARE");
    send_webhook(&app, &screen, None).await;
    let ev = next(&mut ws).await;
    assert_eq!(ev["d"]["streaming"], true);
    assert_eq!(ev["d"]["video"], true);

    // Event einer veralteten Verbindung ändert nichts.
    let stale = track_event("track_unpublished", &room, &bob_id, "PA_0", "CAMERA");
    send_webhook(&app, &stale, None).await;
    assert_eq!(voice_states(&app, &alex, &community).await[0]["video"], true);

    // Kamera aus.
    let uncam = track_event("track_unpublished", &room, &bob_id, "PA_1", "CAMERA");
    send_webhook(&app, &uncam, None).await;
    let ev = next(&mut ws).await;
    assert_eq!(ev["d"]["video"], false);
    assert_eq!(ev["d"]["streaming"], true);

    // Beitritt mit bereits veröffentlichter Kamera (z. B. Reconnect).
    let mut rejoin = participant_event("participant_joined", &room, &bob_id, "PA_2");
    rejoin["participant"]["tracks"] = json!([{ "sid": "TR_1", "type": "VIDEO", "source": "CAMERA" }]);
    send_webhook(&app, &rejoin, None).await;
    let ev = next(&mut ws).await;
    assert_eq!(ev["d"]["video"], true);
    assert_eq!(ev["d"]["streaming"], false);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn channel_audio_settings_and_user_limit(db: PgPool) {
    let app = voice_app(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carol = app.invited_user(&alex, &community, "carol").await;
    let carol_id = app.user_id(&carol).await;
    let room = community.voice_channel.clone();
    let channel_url = format!("/channels/{room}");
    let join = format!("/channels/{room}/voice/join");

    // Standardwerte: 64 kbit/s, Mono, DTX und RED an, kein Limit.
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    let voice =
        detail["channels"].as_array().unwrap().iter().find(|c| c["id"] == room.as_str()).unwrap().clone();
    assert_eq!(voice["audio_bitrate"], 64);
    assert_eq!(voice["audio_stereo"], false);
    assert_eq!(voice["audio_dtx"], true);
    assert_eq!(voice["audio_red"], true);
    assert!(voice["user_limit"].is_null());
    let (status, body) = app.call("POST", &join, Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["audio"], json!({ "bitrate": 64, "stereo": false, "dtx": true, "red": true }));

    // Ohne MANAGE_CHANNELS: 403.
    let (status, _) =
        app.call("PATCH", &channel_url, Some(&bob), Some(json!({ "audio_bitrate": 128 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Ungültige Werte.
    for (patch, code) in [
        (json!({ "audio_bitrate": 31 }), "invalid_bitrate"),
        (json!({ "audio_bitrate": 257 }), "invalid_bitrate"),
        (json!({ "user_limit": 0 }), "invalid_user_limit"),
        (json!({ "user_limit": 100 }), "invalid_user_limit"),
    ] {
        let (status, body) = app.call("PATCH", &channel_url, Some(&alex), Some(patch)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], code);
    }
    // Ton-Felder gibt es nur bei Sprachkanälen.
    let (status, body) = app
        .call(
            "PATCH",
            &format!("/channels/{}", community.text_channel),
            Some(&alex),
            Some(json!({ "audio_dtx": false })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "not_a_voice_channel");

    // Setzen: Werte kommen zurück, andere Felder bleiben.
    let (status, channel) = app
        .call(
            "PATCH",
            &channel_url,
            Some(&alex),
            Some(json!({ "audio_bitrate": 256, "audio_stereo": true, "audio_dtx": false, "audio_red": false,
                         "user_limit": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{channel}");
    assert_eq!(channel["audio_bitrate"], 256);
    assert_eq!(channel["audio_stereo"], true);
    assert_eq!(channel["audio_dtx"], false);
    assert_eq!(channel["audio_red"], false);
    assert_eq!(channel["user_limit"], 1);
    assert_eq!(channel["name"], voice["name"]);
    // Nur den Namen ändern lässt die Ton-Einstellungen in Ruhe.
    let (_, channel) = app.call("PATCH", &channel_url, Some(&alex), Some(json!({ "name": "Musik" }))).await;
    assert_eq!(channel["audio_bitrate"], 256);
    assert_eq!(channel["user_limit"], 1);

    // Das Audit-Log hält die Änderung fest.
    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "channel.update" && e["data"]["audio_bitrate"] == 256),
        "{log}"
    );

    // Beim Beitritt bekommt der Client die Kanal-Einstellungen.
    let (_, body) = app.call("POST", &join, Some(&carol), None).await;
    assert_eq!(body["audio"], json!({ "bitrate": 256, "stereo": true, "dtx": false, "red": false }));

    // Carol sitzt drin → Kanal mit Limit 1 ist voll.
    send_webhook(&app, &participant_event("participant_joined", &room, &carol_id, "PA_1"), None).await;
    wait_for_states(&app, &alex, &community, |s| s.len() == 1).await;
    let (status, body) = app.call("POST", &join, Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "channel_full");
    // Wer schon drin ist, darf neu verbinden; mit MANAGE_CHANNELS kommt man trotzdem hinein.
    assert_eq!(app.call("POST", &join, Some(&carol), None).await.0, StatusCode::OK);
    assert_eq!(app.call("POST", &join, Some(&alex), None).await.0, StatusCode::OK);
    // Bob hat noch ein älteres Token (vor dem Limit geholt): Sein Beitritt wird nicht übernommen.
    let bob_id = app.user_id(&bob).await;
    send_webhook(&app, &participant_event("participant_joined", &room, &bob_id, "PA_2"), None).await;
    let states = voice_states(&app, &alex, &community).await;
    assert_eq!(states.len(), 1, "{states:?}");
    // Mit MANAGE_CHANNELS zählt das Limit auch hier nicht.
    let alex_id = app.user_id(&alex).await;
    send_webhook(&app, &participant_event("participant_joined", &room, &alex_id, "PA_3"), None).await;
    wait_for_states(&app, &alex, &community, |s| s.len() == 2).await;

    // Limit aufheben (null) → Bob kommt wieder hinein.
    let (status, channel) =
        app.call("PATCH", &channel_url, Some(&alex), Some(json!({ "user_limit": null }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(channel["user_limit"].is_null());
    assert_eq!(app.call("POST", &join, Some(&bob), None).await.0, StatusCode::OK);
}
