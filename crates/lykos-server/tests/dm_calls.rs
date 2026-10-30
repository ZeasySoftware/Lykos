//! Integrationstests für Anrufe in DMs: Token und Rechte, Klingeln/Ablehnen/Ablauf, Events nur an die
//! Teilnehmer, Voice-States ohne Community, Blockieren, Gruppe verlassen und Konto löschen im Anruf.
//! Wie `tests/voice.rs` ohne echtes LiveKit: Webhooks werden selbst signiert, RoomService ist ein Mock.

mod common;

use std::time::Duration;

use axum::{
    Json, Router,
    body::Body,
    extract::Path,
    http::{Request, StatusCode, header},
    routing::post,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use common::{PASSWORD, TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use lykos_server::config::{Config, LiveKitConfig};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;

// --- Aufbau --------------------------------------------------------------------

/// Instanz mit Voice; `ring_secs` = Klingeldauer.
fn call_app(db: PgPool, ring_secs: u64) -> TestApp {
    TestApp::with_config(db, Config { call_ring_secs: ring_secs, ..Config::for_tests_with_voice() })
}

/// Wie `call_app`, aber mit RoomService-Mock (zeichnet Methode und Body auf).
async fn call_app_with_mock(db: PgPool) -> (TestApp, mpsc::UnboundedReceiver<(String, Value)>) {
    async fn handle(
        axum::extract::State(tx): axum::extract::State<mpsc::UnboundedSender<(String, Value)>>,
        Path(method): Path<String>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let _ = tx.send((method, body));
        Json(json!({}))
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let router = Router::new().route("/twirp/livekit.RoomService/{method}", post(handle)).with_state(tx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut config = Config { call_ring_secs: 30, ..Config::for_tests_with_voice() };
    config.livekit.as_mut().unwrap().api_url = format!("http://{addr}");
    (TestApp::with_config(db, config), rx)
}

struct People {
    community: TestCommunity,
    alex: String,
    alex_id: String,
    bob: String,
    bob_id: String,
    carol: String,
    carol_id: String,
}

/// Alex, Bob und Carol in einer gemeinsamen Community (dürfen sich also DMs schreiben).
async fn people(app: &TestApp) -> People {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carol = app.invited_user(&alex, &community, "carol").await;
    People {
        alex_id: app.user_id(&alex).await,
        bob_id: app.user_id(&bob).await,
        carol_id: app.user_id(&carol).await,
        community,
        alex,
        bob,
        carol,
    }
}

async fn open_dm(app: &TestApp, token: &str, recipients: &[&str]) -> String {
    let (status, dm) =
        app.call("POST", "/dms", Some(token), Some(json!({ "recipient_ids": recipients }))).await;
    assert!(status == StatusCode::CREATED || status == StatusCode::OK, "{dm}");
    s(&dm["id"]).to_owned()
}

async fn join(app: &TestApp, token: &str, channel: &str) -> (StatusCode, Value) {
    app.call("POST", &format!("/channels/{channel}/voice/join"), Some(token), None).await
}

async fn decline(app: &TestApp, token: &str, channel: &str) -> StatusCode {
    app.call("POST", &format!("/channels/{channel}/call/decline"), Some(token), None).await.0
}

// --- LiveKit-Webhooks ----------------------------------------------------------

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn jwt(claims: &Value, secret: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let input = format!("{header}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn jwt_claims(token: &str) -> Value {
    let payload = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

async fn webhook(app: &TestApp, event: &str, room: &str, identity: &str, sid: &str) {
    let body = json!({
        "event": event,
        "room": { "sid": "RM_test", "name": room },
        "participant": { "sid": sid, "identity": identity, "name": "x", "state": "ACTIVE" },
        "id": "EV_test",
        "createdAt": now().to_string(),
    })
    .to_string();
    let cfg = LiveKitConfig::for_tests();
    let hash = STANDARD.encode(Sha256::digest(body.as_bytes()));
    let auth = jwt(
        &json!({ "iss": cfg.api_key, "nbf": now(), "exp": now() + 300, "sha256": hash }),
        &cfg.api_secret,
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/livekit/webhook")
        .header(header::CONTENT_TYPE, "application/webhook+json")
        .header(header::AUTHORIZATION, auth)
        .body(Body::from(body))
        .unwrap();
    assert_eq!(app.send(req).await.0, StatusCode::OK);
}

// --- Gateway -------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Verbindet und gibt Verbindung und READY-Daten zurück.
async fn connect(app: &TestApp, token: &str) -> (Ws, Value) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    ws.send(tungstenite::Message::Text(json!({ "op": "identify", "token": token }).to_string().into()))
        .await
        .unwrap();
    let ready = next_of(&mut ws, &["READY"]).await;
    (ws, ready["d"].clone())
}

/// Inhalt der Kontrollnachricht von `assert_no_call_events`.
const SENTINEL: &str = "Kontrolle";

/// Nächstes Event einer der Arten `kinds` (andere, z. B. Präsenz, und Kontrollnachrichten werden
/// übersprungen; max. 5 s).
async fn next_of(ws: &mut Ws, kinds: &[&str]) -> Value {
    let fut = async {
        loop {
            if let tungstenite::Message::Text(t) = ws.next().await.unwrap().unwrap() {
                let ev: Value = serde_json::from_str(&t).unwrap();
                let sentinel = ev["t"] == "MESSAGE_CREATE" && ev["d"]["content"] == SENTINEL;
                if !sentinel && kinds.iter().any(|k| ev["t"] == *k) {
                    return ev;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("kein {kinds:?} binnen 5 s"))
}

/// Anruf-relevante Events.
const CALL_EVENTS: &[&str] = &["CALL_UPDATE", "VOICE_STATE_UPDATE", "VOICE_STATE_DELETE", "MESSAGE_CREATE"];

/// Prüft, dass vor einer Kontrollnachricht im Community-Kanal kein Anruf-Event ankam (Events gehen in
/// Reihenfolge raus – was vorher nicht da war, kommt auch nicht mehr).
async fn assert_no_call_events(app: &TestApp, ws: &mut Ws, people: &People) {
    let (status, _) = app
        .call(
            "POST",
            &format!("/channels/{}/messages", people.community.text_channel),
            Some(&people.alex),
            Some(json!({ "content": SENTINEL })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let fut = async {
        loop {
            if let tungstenite::Message::Text(t) = ws.next().await.unwrap().unwrap() {
                let ev: Value = serde_json::from_str(&t).unwrap();
                if ev["t"] == "MESSAGE_CREATE" && ev["d"]["content"] == SENTINEL {
                    return;
                }
                assert!(!CALL_EVENTS.iter().any(|k| ev["t"] == *k), "unerwartetes Anruf-Event: {ev}");
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut).await.expect("Kontrollnachricht kam nicht an");
}

fn ringing(ev: &Value) -> Vec<String> {
    ev["d"]["ringing"].as_array().unwrap().iter().map(|v| s(v).to_owned()).collect()
}

// --- Tests -----------------------------------------------------------------------

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn dm_call_token_and_rights(db: PgPool) {
    let app = call_app(db.clone(), 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;

    // Teilnehmer: Token für den Raum der DM mit allen Quellen, Standard-Ton.
    let (status, body) = join(&app, &p.alex, &dm).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let claims = jwt_claims(s(&body["token"]));
    assert_eq!(claims["sub"], p.alex_id.as_str());
    let video = &claims["video"];
    assert_eq!(video["room"], dm.as_str());
    assert_eq!(video["roomJoin"], true);
    assert_eq!(video["canPublishData"], false);
    assert_eq!(
        video["canPublishSources"],
        json!(["microphone", "screen_share", "screen_share_audio", "camera"])
    );
    assert_eq!(body["audio"], json!({ "bitrate": 64, "stereo": false, "dtx": true, "red": true }));

    // Kein Teilnehmer: 404, ohne Anmeldung 401, unbekannter Kanal 404.
    assert_eq!(join(&app, &p.carol, &dm).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        app.call("POST", &format!("/channels/{dm}/voice/join"), None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(join(&app, &p.alex, &uuid::Uuid::now_v7().to_string()).await.0, StatusCode::NOT_FOUND);
    // Ablehnen/Klingeln nur für Teilnehmer und nur in DMs.
    assert_eq!(decline(&app, &p.carol, &dm).await, StatusCode::NOT_FOUND);
    assert_eq!(decline(&app, &p.alex, &p.community.voice_channel).await, StatusCode::NOT_FOUND);

    // Ohne LiveKit gibt es keine Anrufe.
    let plain = TestApp::new(db);
    let (status, body) = join(&plain, &p.alex, &dm).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "voice_unavailable");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn ringing_decline_accept_and_only_participants(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let (mut bob_ws, _) = connect(&app, &p.bob).await;
    let (mut alex_ws, _) = connect(&app, &p.alex).await;
    let (mut carol_ws, _) = connect(&app, &p.carol).await;

    // Alex ruft an: Bei Bob klingelt es, Alex sieht den Zustand auch.
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    for ws in [&mut bob_ws, &mut alex_ws] {
        let ev = next_of(ws, CALL_EVENTS).await;
        assert_eq!(ev["t"], "CALL_UPDATE");
        assert_eq!(ev["d"]["channel_id"], dm.as_str());
        assert_eq!(ringing(&ev), vec![p.bob_id.clone()]);
        assert_eq!(ev["d"]["caller_id"], p.alex_id.as_str());
    }
    // Carol ist nicht in der DM und erfährt nichts.
    assert_no_call_events(&app, &mut carol_ws, &p).await;

    // Läuft schon ein Klingeln, klingelt ein erneuter Beitritt von Alex nicht noch einmal.
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    // Bob lehnt ab: Klingeln vorbei. Erneutes Ablehnen ist wirkungslos.
    assert_eq!(decline(&app, &p.bob, &dm).await, StatusCode::NO_CONTENT);
    for ws in [&mut bob_ws, &mut alex_ws] {
        let ev = next_of(ws, CALL_EVENTS).await;
        assert_eq!(ev["t"], "CALL_UPDATE");
        assert!(ringing(&ev).is_empty());
        assert_eq!(ev["d"]["caller_id"], Value::Null);
    }
    assert_eq!(decline(&app, &p.bob, &dm).await, StatusCode::NO_CONTENT);
    // Abgelehnt ist nicht verpasst: keine Systemnachricht in der DM.
    let (_, messages) = app.call("GET", &format!("/channels/{dm}/messages"), Some(&p.alex), None).await;
    assert!(messages.as_array().unwrap().is_empty(), "{messages}");

    // Neuer Anruf, Bob nimmt an (Token holen): Klingeln endet, kein neues Klingeln bei Alex.
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(ringing(&next_of(&mut bob_ws, CALL_EVENTS).await), vec![p.bob_id.clone()]);
    assert_eq!(join(&app, &p.bob, &dm).await.0, StatusCode::OK);
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "CALL_UPDATE");
    assert!(ringing(&ev).is_empty());

    // Beide im Raum (Webhooks): Voice-States ohne Community, nur an die Teilnehmer.
    webhook(&app, "participant_joined", &dm, &p.alex_id, "PA_A").await;
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "VOICE_STATE_UPDATE");
    assert_eq!(ev["d"]["community_id"], Value::Null);
    assert_eq!(ev["d"]["channel_id"], dm.as_str());
    assert_eq!(ev["d"]["user_id"], p.alex_id.as_str());
    assert_eq!(next_of(&mut alex_ws, &["VOICE_STATE_UPDATE"]).await["d"]["user_id"], p.alex_id.as_str());
    webhook(&app, "participant_joined", &dm, &p.bob_id, "PA_B").await;
    let ev = next_of(&mut alex_ws, &["VOICE_STATE_UPDATE"]).await;
    assert_eq!(ev["d"]["user_id"], p.bob_id.as_str());
    assert_no_call_events(&app, &mut carol_ws, &p).await;

    // Mute meldet der Client wie im Sprachkanal.
    let (status, _) = app
        .call("POST", "/voice/state", Some(&p.bob), Some(json!({ "self_mute": true, "self_deaf": false })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ev = next_of(&mut alex_ws, &["VOICE_STATE_UPDATE"]).await;
    assert_eq!(ev["d"]["self_mute"], true);
    assert_eq!(ev["d"]["community_id"], Value::Null);

    // Neue Verbindung: READY bringt den laufenden Anruf mit.
    let (_, ready) = connect(&app, &p.bob).await;
    let calls = ready["dm_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["channel_id"], dm.as_str());
    assert_eq!(calls[0]["voice_states"].as_array().unwrap().len(), 2);
    assert_eq!(calls[0]["voice_states"][0]["community_id"], Value::Null);
    assert!(calls[0]["ringing"].as_array().unwrap().is_empty());
    let (_, ready) = connect(&app, &p.carol).await;
    assert!(ready["dm_calls"].as_array().unwrap().is_empty());

    // Wechsel in einen Sprachkanal verlässt den Anruf (ein Nutzer, ein Raum).
    webhook(&app, "participant_joined", &p.community.voice_channel, &p.bob_id, "PA_C").await;
    let ev = next_of(&mut alex_ws, &["VOICE_STATE_DELETE"]).await;
    assert_eq!(ev["d"]["channel_id"], dm.as_str());
    assert_eq!(ev["d"]["community_id"], Value::Null);
    assert_eq!(ev["d"]["user_id"], p.bob_id.as_str());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn ringing_times_out_as_missed_call(db: PgPool) {
    let app = call_app(db, 1);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let (mut bob_ws, _) = connect(&app, &p.bob).await;

    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(ringing(&next_of(&mut bob_ws, CALL_EVENTS).await), vec![p.bob_id.clone()]);
    // Nach Ablauf: Klingeln vorbei und „Anruf verpasst“ von Alex in der DM.
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "CALL_UPDATE");
    assert!(ringing(&ev).is_empty());
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "MESSAGE_CREATE");
    assert_eq!(ev["d"]["kind"], "call_missed");
    assert_eq!(ev["d"]["channel_id"], dm.as_str());
    assert_eq!(ev["d"]["author"]["id"], p.alex_id.as_str());
    assert_eq!(ev["d"]["content"], "");
    // Systemnachrichten lassen sich nicht bearbeiten.
    let (status, _) = app
        .call(
            "PATCH",
            &format!("/messages/{}", s(&ev["d"]["id"])),
            Some(&p.alex),
            Some(json!({ "content": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, ready) = connect(&app, &p.bob).await;
    assert!(ready["dm_calls"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn caller_hangs_up_alone(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let (mut bob_ws, _) = connect(&app, &p.bob).await;

    // Über LiveKit: Alex tritt bei und verlässt den Raum wieder, bevor Bob annimmt.
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(next_of(&mut bob_ws, CALL_EVENTS).await["t"], "CALL_UPDATE");
    webhook(&app, "participant_joined", &dm, &p.alex_id, "PA_A").await;
    assert_eq!(next_of(&mut bob_ws, CALL_EVENTS).await["t"], "VOICE_STATE_UPDATE");
    webhook(&app, "participant_left", &dm, &p.alex_id, "PA_A").await;
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "VOICE_STATE_DELETE");
    assert_eq!(ev["d"]["community_id"], Value::Null);
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "CALL_UPDATE");
    assert!(ringing(&ev).is_empty());
    let ev = next_of(&mut bob_ws, CALL_EVENTS).await;
    assert_eq!(ev["d"]["kind"], "call_missed");

    // Ohne Verbindung zu LiveKit: Der Anrufer bricht über „Ablehnen“ ab (Auflegen vor dem Annehmen).
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(ringing(&next_of(&mut bob_ws, CALL_EVENTS).await), vec![p.bob_id.clone()]);
    assert_eq!(decline(&app, &p.alex, &dm).await, StatusCode::NO_CONTENT);
    assert!(ringing(&next_of(&mut bob_ws, CALL_EVENTS).await).is_empty());
    assert_eq!(next_of(&mut bob_ws, CALL_EVENTS).await["d"]["kind"], "call_missed");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn group_call_rings_everyone_but_blocked(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dave = app.invited_user(&p.alex, &p.community, "dave").await;
    let dave_id = app.user_id(&dave).await;
    let group = open_dm(&app, &p.alex, &[&p.bob_id, &p.carol_id, &dave_id]).await;
    // Dave blockiert Alex (Gruppen-DMs bleiben bestehen) – bei ihm klingelt es nicht.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/relationships/{}", p.alex_id),
            Some(&dave),
            Some(json!({ "type": "blocked" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (mut carol_ws, _) = connect(&app, &p.carol).await;
    let (mut dave_ws, _) = connect(&app, &dave).await;

    assert_eq!(join(&app, &p.alex, &group).await.0, StatusCode::OK);
    let ev = next_of(&mut carol_ws, CALL_EVENTS).await;
    let mut expected = vec![p.bob_id.clone(), p.carol_id.clone()];
    expected.sort();
    assert_eq!(ringing(&ev), expected);
    // Dave sieht den Zustand (Teilnehmer), aber es klingelt nicht bei ihm; beitreten darf er in Gruppen.
    assert_eq!(ringing(&next_of(&mut dave_ws, CALL_EVENTS).await), expected);
    assert_eq!(join(&app, &dave, &group).await.0, StatusCode::OK);

    // Carol nimmt an und ist im Raum; Bob lehnt ab.
    webhook(&app, "participant_joined", &group, &p.alex_id, "PA_A").await;
    webhook(&app, "participant_joined", &group, &p.carol_id, "PA_C").await;
    assert_eq!(decline(&app, &p.bob, &group).await, StatusCode::NO_CONTENT);
    loop {
        let ev = next_of(&mut carol_ws, &["CALL_UPDATE"]).await;
        if ringing(&ev).is_empty() {
            break;
        }
    }
    // Erneut klingeln lassen geht nur im Anruf.
    let ring = format!("/channels/{group}/call/ring");
    let (status, body) = app.call("POST", &ring, Some(&p.bob), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "not_in_call");
    // Carol lässt erneut klingeln: bei Bob und Dave (der blockiert nur Alex), nicht bei denen im Raum.
    assert_eq!(app.call("POST", &ring, Some(&p.carol), None).await.0, StatusCode::NO_CONTENT);
    let ev = next_of(&mut carol_ws, &["CALL_UPDATE"]).await;
    let mut expected = vec![p.bob_id.clone(), dave_id.clone()];
    expected.sort();
    assert_eq!(ringing(&ev), expected);
    assert_eq!(ev["d"]["caller_id"], p.carol_id.as_str());

    // Carol verlässt die Gruppe: Sie fliegt aus dem Anruf, alle sehen es.
    let (status, _) = app.call("DELETE", &format!("/dms/{group}"), Some(&p.carol), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let ev = next_of(&mut dave_ws, &["VOICE_STATE_DELETE"]).await;
    assert_eq!(ev["d"]["user_id"], p.carol_id.as_str());
    // Kein verpasster Anruf in Gruppen (Alex ist noch im Raum, und Gruppen bekommen ohnehin keinen).
    let (_, messages) = app.call("GET", &format!("/channels/{group}/messages"), Some(&p.alex), None).await;
    assert!(messages.as_array().unwrap().is_empty(), "{messages}");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn blocking_ends_the_call(db: PgPool) {
    let (app, mut livekit) = call_app_with_mock(db).await;
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let (mut alex_ws, _) = connect(&app, &p.alex).await;

    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(join(&app, &p.bob, &dm).await.0, StatusCode::OK);
    webhook(&app, "participant_joined", &dm, &p.alex_id, "PA_A").await;
    webhook(&app, "participant_joined", &dm, &p.bob_id, "PA_B").await;

    // Bob blockiert Alex mitten im Anruf: beide fliegen raus, der Raum wird geschlossen.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/relationships/{}", p.alex_id),
            Some(&p.bob),
            Some(json!({ "type": "blocked" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let mut gone = Vec::new();
    while gone.len() < 2 {
        let ev = next_of(&mut alex_ws, &["VOICE_STATE_DELETE"]).await;
        gone.push(s(&ev["d"]["user_id"]).to_owned());
    }
    gone.sort();
    let mut both = vec![p.alex_id.clone(), p.bob_id.clone()];
    both.sort();
    assert_eq!(gone, both);
    let mut methods = Vec::new();
    while methods.len() < 3 {
        let (method, body) =
            tokio::time::timeout(Duration::from_secs(5), livekit.recv()).await.unwrap().unwrap();
        assert_eq!(body["room"], dm.as_str());
        methods.push(method);
    }
    methods.sort();
    assert_eq!(methods, ["DeleteRoom", "RemoveParticipant", "RemoveParticipant"]);

    // Danach gibt es keinen Anruf mehr – in keine Richtung; ein noch gültiges Token wird beim Beitritt entfernt.
    for token in [&p.alex, &p.bob] {
        let (status, body) = join(&app, token, &dm).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "call_not_allowed");
    }
    webhook(&app, "participant_joined", &dm, &p.alex_id, "PA_A2").await;
    let (method, body) = tokio::time::timeout(Duration::from_secs(5), livekit.recv()).await.unwrap().unwrap();
    assert_eq!(method, "RemoveParticipant");
    assert_eq!(body["identity"], p.alex_id.as_str());
    let (_, ready) = connect(&app, &p.alex).await;
    assert!(ready["dm_calls"].as_array().unwrap().is_empty());

    // Blockade aufheben: Anrufen geht wieder.
    let (status, _) = app.call("DELETE", &format!("/relationships/{}", p.alex_id), Some(&p.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn blocking_stops_ringing(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let (mut alex_ws, _) = connect(&app, &p.alex).await;

    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    assert_eq!(ringing(&next_of(&mut alex_ws, CALL_EVENTS).await), vec![p.bob_id.clone()]);
    let (status, _) = app
        .call(
            "PUT",
            &format!("/relationships/{}", p.alex_id),
            Some(&p.bob),
            Some(json!({ "type": "blocked" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let ev = next_of(&mut alex_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "CALL_UPDATE");
    assert!(ringing(&ev).is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn ringing_is_rate_limited(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;

    // Anrufen und selbst abbrechen, immer wieder: Nach 8 Anrufen pro Minute ist Schluss.
    for _ in 0..8 {
        assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
        assert_eq!(decline(&app, &p.alex, &dm).await, StatusCode::NO_CONTENT);
    }
    let (status, body) = join(&app, &p.alex, &dm).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // Annehmen ist kein Anklingeln und geht weiter: Bob ruft an, Alex nimmt an.
    assert_eq!(join(&app, &p.bob, &dm).await.0, StatusCode::OK);
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn account_deletion_mid_call(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let dm = open_dm(&app, &p.alex, &[&p.bob_id]).await;
    let group = open_dm(&app, &p.bob, &[&p.alex_id, &p.carol_id]).await;
    let (mut carol_ws, _) = connect(&app, &p.carol).await;

    // Bob ist im 1:1-Anruf mit Alex; in der Gruppe klingelt es noch von Bob aus.
    assert_eq!(join(&app, &p.bob, &group).await.0, StatusCode::OK);
    assert_eq!(next_of(&mut carol_ws, CALL_EVENTS).await["t"], "CALL_UPDATE");
    assert_eq!(join(&app, &p.alex, &dm).await.0, StatusCode::OK);
    webhook(&app, "participant_joined", &dm, &p.bob_id, "PA_B").await;
    let (mut alex_ws, _) = connect(&app, &p.alex).await;

    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&p.bob), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let ev = next_of(&mut alex_ws, &["VOICE_STATE_DELETE"]).await;
    assert_eq!(ev["d"]["user_id"], p.bob_id.as_str());
    // Das Klingeln von Bob in der Gruppe endet.
    let ev = next_of(&mut carol_ws, CALL_EVENTS).await;
    assert_eq!(ev["t"], "CALL_UPDATE");
    assert!(ringing(&ev).is_empty());
    let (_, ready) = connect(&app, &p.alex).await;
    assert!(ready["dm_calls"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn adding_people_keeps_the_running_call(db: PgPool) {
    let app = call_app(db, 30);
    let p = people(&app).await;
    let group = open_dm(&app, &p.alex, &[&p.bob_id, &p.carol_id]).await;
    // Dave kommt erst später dazu.
    let dave = app.invited_user(&p.alex, &p.community, "dave").await;
    let dave_id = app.user_id(&dave).await;
    let (mut dave_ws, _) = connect(&app, &dave).await;
    let (mut bob_ws, _) = connect(&app, &p.bob).await;

    // Alex ruft an und ist im Raum; bei Bob und Carol klingelt es.
    assert_eq!(join(&app, &p.alex, &group).await.0, StatusCode::OK);
    webhook(&app, "participant_joined", &group, &p.alex_id, "PA_A").await;
    next_of(&mut bob_ws, &["VOICE_STATE_UPDATE"]).await;

    let (status, dm) = app
        .call(
            "POST",
            &format!("/dms/{group}/recipients"),
            Some(&p.bob),
            Some(json!({ "recipient_ids": [dave_id] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dm}");
    // Dave bekommt die Gruppe und den laufenden Anruf mit (Teilnehmer + Klingeln), ohne dass es bei ihm klingelt.
    assert_eq!(next_of(&mut dave_ws, &["DM_CREATE"]).await["d"]["id"], group.as_str());
    let vs = next_of(&mut dave_ws, &["VOICE_STATE_UPDATE"]).await;
    assert_eq!(vs["d"]["user_id"], p.alex_id.as_str());
    let call = next_of(&mut dave_ws, &["CALL_UPDATE"]).await;
    assert!(!ringing(&call).contains(&dave_id));
    assert!(!ringing(&call).is_empty());
    // Der Anruf läuft weiter, Dave kann beitreten.
    let in_call: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM voice_states WHERE channel_id = $1::uuid)")
            .bind(&group)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(in_call);
    assert_eq!(join(&app, &dave, &group).await.0, StatusCode::OK);
}
