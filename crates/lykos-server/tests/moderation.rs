//! Integrationstests für Banns, den AFK-Kanal und die anpassbare Willkommensnachricht.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::{TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use lykos_server::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

const KICK_MEMBERS: i64 = 1 << 10;
const BAN_MEMBERS: i64 = 1 << 17;

async fn create_role(
    app: &TestApp,
    token: &str,
    community: &TestCommunity,
    name: &str,
    perms: i64,
) -> String {
    let (status, role) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(token),
            Some(json!({ "name": name, "permissions": perms })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    s(&role["id"]).to_owned()
}

async fn give_role(app: &TestApp, owner: &str, community: &TestCommunity, user_id: &str, role: &str) {
    let path = format!("/communities/{}/members/{user_id}/roles/{role}", community.id);
    assert_eq!(app.call("PUT", &path, Some(owner), None).await.0, StatusCode::NO_CONTENT);
}

async fn ban(
    app: &TestApp,
    token: &str,
    community: &TestCommunity,
    target: &str,
    body: Value,
) -> (StatusCode, Value) {
    app.call("PUT", &format!("/communities/{}/bans/{target}", community.id), Some(token), Some(body)).await
}

async fn invite(app: &TestApp, token: &str, community: &TestCommunity) -> String {
    let (status, invite) = app
        .call("POST", &format!("/communities/{}/invites", community.id), Some(token), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{invite}");
    s(&invite["code"]).to_owned()
}

async fn patch_community(
    app: &TestApp,
    token: &str,
    community: &TestCommunity,
    body: Value,
) -> (StatusCode, Value) {
    app.call("PATCH", &format!("/communities/{}", community.id), Some(token), Some(body)).await
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
    assert_eq!(next_event(&mut ws, "READY").await["t"], "READY");
    ws
}

/// Nächstes Event der Art `kind` vom Gateway (andere werden übersprungen, max. 5 s).
async fn next_event(ws: &mut Ws, kind: &str) -> Value {
    let fut = async {
        loop {
            if let tungstenite::Message::Text(t) = ws.next().await.unwrap().unwrap() {
                let v = serde_json::from_str::<Value>(&t).unwrap();
                if v["t"] == kind {
                    return v;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("kein {kind} binnen 5 s"))
}

// --- Banns -------------------------------------------------------------------

/// Bannen braucht `BAN_MEMBERS`, folgt der Rollenhierarchie und trifft nie den Besitzer oder sich selbst.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn ban_requires_permission_and_hierarchy(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carl = app.invited_user(&alex, &community, "carl").await;
    let dora = app.invited_user(&alex, &community, "dora").await;
    let erik = app.invited_user(&alex, &community, "erik").await;
    let [alex_id, bob_id, carl_id, dora_id, erik_id] = [
        app.user_id(&alex).await,
        app.user_id(&bob).await,
        app.user_id(&carl).await,
        app.user_id(&dora).await,
        app.user_id(&erik).await,
    ];

    // Ohne Recht: verboten (auch die Bannliste).
    assert_eq!(ban(&app, &erik, &community, &carl_id, json!({})).await.0, StatusCode::FORBIDDEN);
    let bans_path = format!("/communities/{}/bans", community.id);
    assert_eq!(app.call("GET", &bans_path, Some(&erik), None).await.0, StatusCode::FORBIDDEN);

    // Admins (1) > Mods (2); Bob und Carl sind Mods, Dora Admin, Erik nur Standardrolle.
    let admins = create_role(&app, &alex, &community, "Admins", BAN_MEMBERS).await;
    let mods = create_role(&app, &alex, &community, "Mods", BAN_MEMBERS).await;
    for (user, role) in [(&bob_id, &mods), (&carl_id, &mods), (&dora_id, &admins)] {
        give_role(&app, &alex, &community, user, role).await;
    }

    assert_eq!(ban(&app, &bob, &community, &carl_id, json!({})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ban(&app, &bob, &community, &dora_id, json!({})).await.0, StatusCode::FORBIDDEN);
    // Nie den Besitzer, nie sich selbst – auch nicht als Besitzer.
    assert_eq!(ban(&app, &dora, &community, &alex_id, json!({})).await.0, StatusCode::FORBIDDEN);
    let (status, body) = ban(&app, &bob, &community, &bob_id, json!({})).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "cannot_ban_self"));
    let (status, body) = ban(&app, &alex, &community, &alex_id, json!({})).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "cannot_ban_self"));
    // Ungültige Werte.
    let (status, body) = ban(&app, &bob, &community, &erik_id, json!({ "reason": "x".repeat(513) })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_reason"));
    let (status, _) =
        ban(&app, &bob, &community, &erik_id, json!({ "delete_message_seconds": 7 * 86_400 + 1 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let unknown = uuid::Uuid::now_v7().to_string();
    assert_eq!(ban(&app, &bob, &community, &unknown, json!({})).await.0, StatusCode::NOT_FOUND);

    // Darunter geht: Bob bannt Erik, Dora bannt Carl, der Besitzer Dora.
    assert_eq!(ban(&app, &bob, &community, &erik_id, json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ban(&app, &dora, &community, &carl_id, json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ban(&app, &alex, &community, &dora_id, json!({})).await.0, StatusCode::NO_CONTENT);
    let (_, list) = app.call("GET", &bans_path, Some(&bob), None).await;
    assert_eq!(list.as_array().unwrap().len(), 3, "{list}");
}

/// Die Migration gibt `BAN_MEMBERS` allen Rollen, die schon `KICK_MEMBERS` haben.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn migration_grants_ban_to_kick_roles(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let mods = create_role(&app, &alex, &community, "Mods", KICK_MEMBERS).await;
    let helfer = create_role(&app, &alex, &community, "Helfer", 0).await;
    sqlx::raw_sql(include_str!("../migrations/0024_moderation.sql")).execute(&db).await.unwrap();
    let perms = |id: String| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT permissions FROM roles WHERE id = $1::uuid")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(perms(mods).await, KICK_MEMBERS | BAN_MEMBERS);
    assert_eq!(perms(helfer).await, 0);
}

/// Gebannte fliegen sofort raus, kommen über Einladungen nicht zurück; nach dem Entbannen schon.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn banned_users_cannot_rejoin_until_unbanned(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let alex_id = app.user_id(&alex).await;

    let mut alex_ws = connect_gateway(&app, &alex).await;
    let (status, body) = ban(&app, &alex, &community, &bob_id, json!({ "reason": "  Spam  " })).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let removed = next_event(&mut alex_ws, "MEMBER_REMOVE").await;
    assert_eq!(removed["d"]["user_id"], bob_id.as_str());

    // Kein Mitglied mehr.
    let community_path = format!("/communities/{}", community.id);
    assert_eq!(app.call("GET", &community_path, Some(&bob), None).await.0, StatusCode::NOT_FOUND);
    let (_, members) = app.call("GET", &format!("{community_path}/members"), Some(&alex), None).await;
    assert!(members.as_array().unwrap().iter().all(|m| m["user"]["id"] != bob_id.as_str()));

    // Bannliste mit Grund und Moderator.
    let (status, list) = app.call("GET", &format!("{community_path}/bans"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["user"]["username"], "bob");
    assert_eq!(list[0]["reason"], "Spam");
    assert_eq!(list[0]["banned_by"]["id"], alex_id.as_str());
    assert!(list[0]["created_at"].is_string());

    // Einladung annehmen: 403 banned.
    let code = invite(&app, &alex, &community).await;
    let (status, body) = app.call("POST", &format!("/invites/{code}/accept"), Some(&bob), None).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "banned"));

    // Audit-Log.
    let (_, log) = app.call("GET", &format!("{community_path}/audit-log"), Some(&alex), None).await;
    assert_eq!(log[1]["action"], "member.ban");
    assert_eq!(log[1]["target_id"], bob_id.as_str());
    assert_eq!(log[1]["data"]["reason"], "Spam");

    // Datenexport nennt den Bann bei beiden Seiten.
    let (_, export) = app.call("GET", "/auth/me/export", Some(&bob), None).await;
    assert_eq!(export["community_bans"]["received"][0]["community_name"], "Rudel");
    let (_, export) = app.call("GET", "/auth/me/export", Some(&alex), None).await;
    assert_eq!(export["community_bans"]["issued"][0]["user_id"], bob_id.as_str());

    // Entbannen (zweimal: 404), danach klappt der Beitritt wieder.
    let unban_path = format!("{community_path}/bans/{bob_id}");
    assert_eq!(app.call("DELETE", &unban_path, Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.call("DELETE", &unban_path, Some(&alex), None).await.0, StatusCode::NOT_FOUND);
    let (_, log) = app.call("GET", &format!("{community_path}/audit-log"), Some(&alex), None).await;
    assert_eq!(log[0]["action"], "member.unban");
    assert_eq!(
        app.call("POST", &format!("/invites/{code}/accept"), Some(&bob), None).await.0,
        StatusCode::OK
    );

    // Ehemalige Mitglieder lassen sich ebenfalls bannen (z. B. nach dem Verlassen).
    assert_eq!(
        app.call("DELETE", &format!("{community_path}/members/{bob_id}"), Some(&bob), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(ban(&app, &alex, &community, &bob_id, json!({})).await.0, StatusCode::NO_CONTENT);
    let (status, _) = app.call("POST", &format!("/invites/{code}/accept"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Konto-Löschung des Gebannten räumt den Bann mit ab.
    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&bob), Some(json!({ "password": common::PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, list) = app.call("GET", &format!("{community_path}/bans"), Some(&alex), None).await;
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

/// Bannen mit Löschzeitraum entfernt nur die Nachrichten des Nutzers aus diesem Zeitraum – mit Events.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn ban_deletes_recent_messages(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    let messages_path = format!("/channels/{}/messages", community.text_channel);

    let mut ids = Vec::new();
    for text in ["eins", "zwei", "alt"] {
        let (status, m) =
            app.call("POST", &messages_path, Some(&bob), Some(json!({ "content": text }))).await;
        assert_eq!(status, StatusCode::CREATED, "{m}");
        ids.push(s(&m["id"]).to_owned());
    }
    app.call("POST", &messages_path, Some(&alex), Some(json!({ "content": "bleibt" }))).await;
    // Eine Nachricht ist zwei Tage alt – außerhalb des Zeitraums.
    sqlx::query("UPDATE messages SET created_at = now() - interval '2 days' WHERE id = $1::uuid")
        .bind(&ids[2])
        .execute(&db)
        .await
        .unwrap();

    let mut alex_ws = connect_gateway(&app, &alex).await;
    let (status, body) =
        ban(&app, &alex, &community, &bob_id, json!({ "delete_message_seconds": 86_400 })).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let mut deleted = Vec::new();
    // Zwei eigene Nachrichten plus Bobs Beitrittsmeldung im Systemkanal.
    for _ in 0..3 {
        let ev = next_event(&mut alex_ws, "MESSAGE_DELETE").await;
        assert_eq!(ev["d"]["channel_id"], community.text_channel.as_str());
        assert_eq!(ev["d"]["community_id"], community.id.as_str());
        deleted.push(s(&ev["d"]["id"]).to_owned());
    }
    assert!(deleted.contains(&ids[0]) && deleted.contains(&ids[1]), "{deleted:?}");

    let (_, msgs) = app.call("GET", &messages_path, Some(&alex), None).await;
    let contents: Vec<&str> = msgs.as_array().unwrap().iter().map(|m| s(&m["content"])).collect();
    assert_eq!(contents.len(), 2, "{msgs}");
    assert!(contents.contains(&"alt") && contents.contains(&"bleibt"), "{contents:?}");
}

// --- AFK-Kanal ---------------------------------------------------------------

/// AFK-Kanal: nur ein Sprachkanal der eigenen Community, nur erlaubte Zeiten, `MANAGE_COMMUNITY`.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn afk_settings_validation(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let other = app.community(&alex, "Andere").await;
    let bob = app.invited_user(&alex, &community, "bob").await;

    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert!(detail["community"]["afk_channel_id"].is_null());
    assert_eq!(detail["community"]["afk_timeout_seconds"], 300);

    let set = json!({ "afk_channel_id": community.voice_channel, "afk_timeout_seconds": 900 });
    assert_eq!(patch_community(&app, &bob, &community, set.clone()).await.0, StatusCode::FORBIDDEN);
    for bad in [
        json!({ "afk_channel_id": community.text_channel }),
        json!({ "afk_channel_id": other.voice_channel }),
        json!({ "afk_channel_id": uuid::Uuid::now_v7() }),
    ] {
        let (status, body) = patch_community(&app, &alex, &community, bad).await;
        assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_channel"));
    }
    for bad in [0, 120, 7200] {
        let (status, _) =
            patch_community(&app, &alex, &community, json!({ "afk_timeout_seconds": bad })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    let (status, c) = patch_community(&app, &alex, &community, set).await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["afk_channel_id"], community.voice_channel.as_str());
    assert_eq!(c["afk_timeout_seconds"], 900);
    // Andere Felder lassen die AFK-Einstellung stehen; `null` entfernt den Kanal.
    let (_, c) = patch_community(&app, &alex, &community, json!({ "name": "Rudel 2" })).await;
    assert_eq!(c["afk_channel_id"], community.voice_channel.as_str());
    let (_, c) = patch_community(&app, &alex, &community, json!({ "afk_channel_id": null })).await;
    assert!(c["afk_channel_id"].is_null());
    assert_eq!(c["afk_timeout_seconds"], 900);

    // Wird der Kanal gelöscht, fällt die Einstellung weg.
    patch_community(&app, &alex, &community, json!({ "afk_channel_id": community.voice_channel })).await;
    app.call("DELETE", &format!("/channels/{}", community.voice_channel), Some(&alex), None).await;
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert!(detail["community"]["afk_channel_id"].is_null());
}

/// Im AFK-Kanal erlaubt das Token nur Zuhören – auch dem Besitzer.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn afk_channel_denies_speaking(db: PgPool) {
    let app = TestApp::with_config(db, Config::for_tests_with_voice());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let (status, afk) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "AFK", "kind": "voice" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{afk}");
    let afk_id = s(&afk["id"]).to_owned();
    patch_community(&app, &alex, &community, json!({ "afk_channel_id": afk_id })).await;

    let sources = |channel: String, token: String| {
        let app = &app;
        async move {
            let (status, body) =
                app.call("POST", &format!("/channels/{channel}/voice/join"), Some(&token), None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let payload = s(&body["token"]).split('.').nth(1).unwrap().to_owned();
            let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
            claims["video"].clone()
        }
    };
    for token in [&bob, &alex] {
        let video = sources(afk_id.clone(), token.clone()).await;
        assert_eq!(video["roomJoin"], true);
        assert_eq!(video["canSubscribe"], true);
        assert_eq!(video["canPublish"], false, "{video}");
        assert!(video["canPublishSources"].as_array().is_none_or(Vec::is_empty), "{video}");
    }
    // Andere Sprachkanäle bleiben normal.
    let video = sources(community.voice_channel.clone(), bob.clone()).await;
    assert_eq!(
        video["canPublishSources"],
        json!(["microphone", "screen_share", "screen_share_audio", "camera"])
    );
}

// --- Willkommensnachricht ----------------------------------------------------

async fn join_messages(app: &TestApp, token: &str, community: &TestCommunity) -> Vec<Value> {
    let (_, msgs) =
        app.call("GET", &format!("/channels/{}/messages", community.text_channel), Some(token), None).await;
    msgs.as_array().unwrap().iter().filter(|m| m["kind"] == "member_join").cloned().collect()
}

/// Neue Communities haben ihren ersten Textkanal als Systemkanal; eigener Text mit Platzhaltern und
/// Schalter für Beitrittsmeldungen.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn custom_welcome_message(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert_eq!(detail["community"]["system_channel_id"], community.text_channel.as_str());
    assert_eq!(detail["community"]["join_messages_enabled"], true);
    assert!(detail["community"]["welcome_message"].is_null());

    // Standard: leerer Inhalt (Client wählt einen zufälligen Satz).
    app.invited_user(&alex, &community, "bob").await;
    let msgs = join_messages(&app, &alex, &community).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["content"], "");

    // Zu lang bzw. ohne Recht.
    let (status, body) =
        patch_community(&app, &alex, &community, json!({ "welcome_message": "x".repeat(501) })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_welcome_message"));

    let (status, c) = patch_community(
        &app,
        &alex,
        &community,
        json!({ "welcome_message": "  Hallo {user}, willkommen bei **{community}**!  " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["welcome_message"], "Hallo {user}, willkommen bei **{community}**!");
    let carl = app.invited_user(&alex, &community, "carl").await;
    let carl_id = app.user_id(&carl).await;
    let msgs = join_messages(&app, &alex, &community).await;
    assert_eq!(msgs[0]["content"], "Hallo @carl, willkommen bei **Rudel**!");
    assert_eq!(msgs[0]["author"]["id"], carl_id.as_str());
    assert_eq!(msgs[0]["mentions"], json!([carl_id]));

    // Schalter aus: keine Meldung, Text bleibt gespeichert.
    let (_, c) = patch_community(&app, &alex, &community, json!({ "join_messages_enabled": false })).await;
    assert_eq!(c["join_messages_enabled"], false);
    assert!(c["welcome_message"].is_string());
    app.invited_user(&alex, &community, "dora").await;
    assert_eq!(join_messages(&app, &alex, &community).await.len(), 2);

    // Wieder an, Text entfernt: zurück zu den Standardsätzen.
    patch_community(&app, &alex, &community, json!({ "join_messages_enabled": true, "welcome_message": "" }))
        .await;
    app.invited_user(&alex, &community, "erik").await;
    let msgs = join_messages(&app, &alex, &community).await;
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["content"], "");
    assert_eq!(msgs[0]["mentions"], json!([]));

    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    let update = log.as_array().unwrap().iter().find(|e| e["action"] == "community.update").unwrap();
    assert!(update["data"].get("welcome_message").is_some(), "{update}");
}
