//! Kanal-Kategorien: CRUD mit `MANAGE_CHANNELS`, Zuordnung von Kanälen, Sichtbarkeit und Gateway-Events.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::StatusCode;
use common::{TestApp, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

const VIEW_CHANNEL: i64 = 1 << 0;
const MANAGE_CHANNELS: i64 = 1 << 8;

async fn detail(app: &TestApp, token: &str, community: &str) -> Value {
    let (status, body) = app.call("GET", &format!("/communities/{community}"), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn category_names(detail: &Value) -> Vec<String> {
    detail["categories"].as_array().unwrap().iter().map(|c| s(&c["name"]).to_owned()).collect()
}

fn channel<'a>(detail: &'a Value, id: &str) -> &'a Value {
    detail["channels"].as_array().unwrap().iter().find(|c| c["id"] == id).expect("Kanal fehlt")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn category_crud_and_channel_assignment(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let path = format!("/communities/{}/categories", community.id);

    // Bestehende Kanäle haben keine Kategorie; neue Communities keine Kategorien.
    let d = detail(&app, &alex, &community.id).await;
    assert_eq!(d["categories"], json!([]));
    assert!(channel(&d, &community.text_channel)["category_id"].is_null());

    let (status, games) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "  Spiele  " }))).await;
    assert_eq!(status, StatusCode::CREATED, "{games}");
    assert_eq!(games["name"], "Spiele");
    assert_eq!(games["position"], 0);
    let (_, talk) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Reden" }))).await;
    assert_eq!(talk["position"], 1);
    let (status, _) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "   " }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Kanal direkt in einer Kategorie anlegen.
    let (status, ch) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "Minecraft", "kind": "text", "category_id": games["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ch}");
    assert_eq!(ch["category_id"], games["id"]);

    // Bestehenden Kanal verschieben und wieder herausnehmen.
    let voice = format!("/channels/{}", community.voice_channel);
    let (status, moved) =
        app.call("PATCH", &voice, Some(&alex), Some(json!({ "category_id": talk["id"] }))).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["category_id"], talk["id"]);
    let (_, moved) = app.call("PATCH", &voice, Some(&alex), Some(json!({ "name": "Lounge" }))).await;
    assert_eq!(moved["category_id"], talk["id"], "Ohne Feld bleibt die Kategorie");
    let (_, moved) = app.call("PATCH", &voice, Some(&alex), Some(json!({ "category_id": null }))).await;
    assert!(moved["category_id"].is_null());

    // Umbenennen und umsortieren.
    let (status, renamed) = app
        .call(
            "PATCH",
            &format!("/categories/{}", s(&games["id"])),
            Some(&alex),
            Some(json!({ "name": "Games" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "Games");
    let (status, ordered) = app
        .call(
            "PATCH",
            &format!("{path}/order"),
            Some(&alex),
            Some(json!({ "category_ids": [talk["id"], games["id"]] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ordered}");
    assert_eq!(category_names(&json!({ "categories": ordered })), ["Reden", "Games"]);
    let (status, _) = app
        .call("PATCH", &format!("{path}/order"), Some(&alex), Some(json!({ "category_ids": [talk["id"]] })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Unvollständige Liste");
    assert_eq!(category_names(&detail(&app, &alex, &community.id).await), ["Reden", "Games"]);

    // Löschen: Kanäle bleiben, ohne Kategorie.
    let (status, _) =
        app.call("DELETE", &format!("/categories/{}", s(&games["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let d = detail(&app, &alex, &community.id).await;
    assert_eq!(category_names(&d), ["Reden"]);
    assert!(channel(&d, s(&ch["id"]))["category_id"].is_null());
    let (status, _) =
        app.call("DELETE", &format!("/categories/{}", s(&games["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Im Audit-Log.
    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| s(&e["action"])).collect();
    for action in ["category.create", "category.update", "category.reorder", "category.delete"] {
        assert!(actions.contains(&action), "{action} fehlt in {actions:?}");
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn categories_need_manage_channels_and_same_community(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let other = app.community(&alex, "Andere").await;
    let path = format!("/communities/{}/categories", community.id);

    let (_, cat) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Team" }))).await;
    let cat_path = format!("/categories/{}", s(&cat["id"]));

    let (status, _) = app.call("POST", &path, Some(&bob), Some(json!({ "name": "Meins" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app.call("PATCH", &cat_path, Some(&bob), Some(json!({ "name": "X" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app.call("DELETE", &cat_path, Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .call("PATCH", &format!("{path}/order"), Some(&bob), Some(json!({ "category_ids": [cat["id"]] })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Fremde Community: Kategorie gibt es dort nicht.
    let (status, body) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", other.id),
            Some(&alex),
            Some(json!({ "name": "x", "kind": "text", "category_id": cat["id"] })),
        )
        .await;
    assert_eq!((status, body["error"].clone()), (StatusCode::BAD_REQUEST, json!("invalid_category")));
    let (status, body) = app
        .call(
            "PATCH",
            &format!("/channels/{}", other.text_channel),
            Some(&alex),
            Some(json!({ "category_id": cat["id"] })),
        )
        .await;
    assert_eq!((status, body["error"].clone()), (StatusCode::BAD_REQUEST, json!("invalid_category")));

    // Nicht-Mitglieder erfahren nichts über die Kategorie.
    let carol = app.invited_user(&alex, &other, "carol").await;
    let (status, _) = app.call("PATCH", &cat_path, Some(&carol), Some(json!({ "name": "X" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Mit MANAGE_CHANNELS über eine Rolle geht es.
    let (_, role) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(&alex),
            Some(json!({ "name": "Kanalpflege", "permissions": MANAGE_CHANNELS })),
        )
        .await;
    let bob_id = app.user_id(&bob).await;
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{}/members/{bob_id}/roles/{}", community.id, s(&role["id"])),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("PATCH", &cat_path, Some(&bob), Some(json!({ "name": "Team 2" }))).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn members_only_see_categories_with_visible_channels(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let path = format!("/communities/{}/categories", community.id);

    let (_, empty) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Leer" }))).await;
    let (_, secret) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Geheim" }))).await;
    let (_, open) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Offen" }))).await;
    let create = |name: &str, cat: &Value| json!({ "name": name, "kind": "text", "category_id": cat["id"] });
    let channels = format!("/communities/{}/channels", community.id);
    let (_, hidden) = app.call("POST", &channels, Some(&alex), Some(create("mods", &secret))).await;
    app.call("POST", &channels, Some(&alex), Some(create("plaudern", &open))).await;
    // Kanal in „Geheim“ für die Standardrolle unsichtbar.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/channels/{}/overrides/{}", s(&hidden["id"]), community.default_role),
            Some(&alex),
            Some(json!({ "allow": 0, "deny": VIEW_CHANNEL })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Besitzer sieht alle, Bob nur die mit sichtbarem Kanal.
    assert_eq!(category_names(&detail(&app, &alex, &community.id).await), ["Leer", "Geheim", "Offen"]);
    let d = detail(&app, &bob, &community.id).await;
    assert_eq!(category_names(&d), ["Offen"]);
    assert!(d["channels"].as_array().unwrap().iter().all(|c| c["id"] != hidden["id"]));
    let _ = empty;
}

// --- Gateway ---------------------------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
    next_of(&mut ws, &["READY"]).await;
    ws
}

/// Nächstes Event einer der Arten `kinds` (andere werden übersprungen); `None` nach `wait`.
async fn try_next_of(ws: &mut Ws, kinds: &[&str], wait: Duration) -> Option<Value> {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    let event: Value = serde_json::from_str(&t).unwrap();
                    if kinds.iter().any(|k| event["t"] == *k) {
                        return event;
                    }
                }
                Some(Ok(_)) => continue,
                other => panic!("Verbindung beendet: {other:?}"),
            }
        }
    };
    tokio::time::timeout(wait, fut).await.ok()
}

async fn next_of(ws: &mut Ws, kinds: &[&str]) -> Value {
    try_next_of(ws, kinds, Duration::from_secs(10)).await.expect("Event kam nicht")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn category_events_follow_visibility(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let addr = serve(&app).await;
    let mut alex_ws = connect(addr, &alex).await;
    let mut bob_ws = connect(addr, &bob).await;
    let kinds = ["CATEGORY_CREATE", "CATEGORY_UPDATE", "CATEGORY_DELETE"];

    // Leere Kategorie: nur wer Kanäle verwaltet, erfährt davon.
    let path = format!("/communities/{}/categories", community.id);
    let (_, cat) = app.call("POST", &path, Some(&alex), Some(json!({ "name": "Spiele" }))).await;
    let ev = next_of(&mut alex_ws, &kinds).await;
    assert_eq!((ev["t"].clone(), ev["d"]["id"].clone()), (json!("CATEGORY_CREATE"), cat["id"].clone()));
    assert!(try_next_of(&mut bob_ws, &kinds, Duration::from_millis(400)).await.is_none());

    // Mit sichtbarem Kanal darin bekommt Bob Umbenennungen.
    let (status, _) = app
        .call(
            "PATCH",
            &format!("/channels/{}", community.text_channel),
            Some(&alex),
            Some(json!({ "category_id": cat["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let ev = next_of(&mut bob_ws, &["CHANNEL_UPDATE"]).await;
    assert_eq!(ev["d"]["category_id"], cat["id"]);
    app.call(
        "PATCH",
        &format!("/categories/{}", s(&cat["id"])),
        Some(&alex),
        Some(json!({ "name": "Games" })),
    )
    .await;
    let ev = next_of(&mut bob_ws, &kinds).await;
    assert_eq!((ev["t"].clone(), ev["d"]["name"].clone()), (json!("CATEGORY_UPDATE"), json!("Games")));

    // Löschen geht an alle.
    app.call("DELETE", &format!("/categories/{}", s(&cat["id"])), Some(&alex), None).await;
    let ev = next_of(&mut bob_ws, &kinds).await;
    assert_eq!((ev["t"].clone(), ev["d"]["id"].clone()), (json!("CATEGORY_DELETE"), cat["id"].clone()));
    let ev = next_of(&mut alex_ws, &["CATEGORY_DELETE"]).await;
    assert_eq!(ev["d"]["community_id"], community.id.as_str());
}
