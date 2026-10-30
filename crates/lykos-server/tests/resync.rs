//! `RESYNC` nach verpassten Events: enthält den aktuellen Präsenz-Stand (Paket E).

mod common;

use std::{net::SocketAddr, time::Duration};

use common::TestApp;
use futures_util::{SinkExt, StreamExt};
use lykos_server::{config::Config, events::Scope};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: SocketAddr, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    let identify = json!({ "op": "identify", "token": token }).to_string();
    ws.send(tungstenite::Message::Text(identify.into())).await.unwrap();
    assert!(next_of(&mut ws, "READY").await["user"]["username"].is_string());
    ws
}

async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    let event: Value = serde_json::from_str(&t).unwrap();
                    if event["t"] == kind {
                        return event["d"].clone();
                    }
                }
                Some(Ok(_)) => continue,
                other => panic!("Verbindung beendet: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), fut).await.expect("Event kam nicht")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn resync_carries_presences(db: PgPool) {
    let state = lykos_server::state(db.clone(), Config::for_tests());
    let app = TestApp { router: lykos_server::app(state.clone()).unwrap(), db };
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut alex_ws = connect(addr, &alex).await;
    let _bob_ws = connect(addr, &bob).await;
    let update = next_of(&mut alex_ws, "PRESENCE_UPDATE").await;
    assert_eq!(update["status"], "online");

    // Bus überfluten, ohne dazwischen nachzugeben → Alex' Verbindung hinkt hinterher.
    for _ in 0..5000 {
        state.events.publish(Scope::User(Uuid::nil()), "TEST", json!({}));
    }

    let resync = next_of(&mut alex_ws, "RESYNC").await;
    let bob_status = resync["presences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["user_id"] == bob_id.as_str())
        .map(|p| p["status"].clone());
    assert_eq!(bob_status, Some(json!("online")));
}
