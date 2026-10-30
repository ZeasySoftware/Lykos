//! Integrationstests für Nachrichten-Features: Antworten, Reaktionen, @Erwähnungen, Anpinnen.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{TestApp, TestCommunity, s};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite;

const VIEW_CHANNEL: i64 = 1 << 0;
const MANAGE_MESSAGES: i64 = 1 << 2;
const ADD_REACTIONS: i64 = 1 << 14;
const MENTION_EVERYONE: i64 = 1 << 15;

/// Prozentkodiert einen Pfadabschnitt (für Emojis in der URL).
fn enc(segment: &str) -> String {
    segment.bytes().map(|b| format!("%{b:02X}")).collect()
}

async fn setup(app: &TestApp) -> (String, String, TestCommunity) {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    (alex, bob, community)
}

async fn send(app: &TestApp, token: &str, channel: &str, body: Value) -> (StatusCode, Value) {
    app.call("POST", &format!("/channels/{channel}/messages"), Some(token), Some(body)).await
}

async fn post(app: &TestApp, token: &str, channel: &str, content: &str) -> Value {
    let (status, msg) = send(app, token, channel, json!({ "content": content })).await;
    assert_eq!(status, StatusCode::CREATED, "{msg}");
    msg
}

async fn messages(app: &TestApp, token: &str, channel: &str) -> Vec<Value> {
    let (status, list) = app.call("GET", &format!("/channels/{channel}/messages"), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list.as_array().unwrap().clone()
}

async fn find(app: &TestApp, token: &str, channel: &str, id: &str) -> Value {
    messages(app, token, channel).await.into_iter().find(|m| m["id"] == id).expect("Nachricht fehlt")
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

async fn second_text_channel(app: &TestApp, owner: &str, community: &TestCommunity, name: &str) -> String {
    let (status, channel) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(owner),
            Some(json!({ "name": name, "kind": "text" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    s(&channel["id"]).to_owned()
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn replies_reference_the_original(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;

    let original = post(&app, &alex, general, &"x".repeat(300)).await;
    assert!(original["reply_to_id"].is_null());
    assert!(original["referenced_message"].is_null());
    let original_id = s(&original["id"]).to_owned();

    let (status, reply) =
        send(&app, &bob, general, json!({ "content": "Stimmt!", "reply_to_id": original_id })).await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");
    assert_eq!(reply["reply_to_id"], original_id.as_str());
    let reference = &reply["referenced_message"];
    assert_eq!(reference["id"], original_id.as_str());
    assert_eq!(reference["author"]["username"], "alex");
    assert_eq!(reference["content"].as_str().unwrap().chars().count(), 200, "Vorschau ist gekürzt");
    assert_eq!(reference["has_attachments"], false);

    // Unbekannte Nachricht oder Nachricht aus einem anderen Kanal: invalid_reply.
    let other = second_text_channel(&app, &alex, &community, "anderer").await;
    let foreign = post(&app, &alex, &other, "woanders").await;
    for target in [s(&foreign["id"]).to_owned(), uuid::Uuid::now_v7().to_string()] {
        let (status, body) =
            send(&app, &bob, general, json!({ "content": "?", "reply_to_id": target })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_reply");
    }

    // Original löschen: Antwort bleibt, Verweis zeigt auf eine gelöschte Nachricht.
    let (status, _) = app.call("DELETE", &format!("/messages/{original_id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let reply = find(&app, &bob, general, s(&reply["id"])).await;
    assert_eq!(reply["reply_to_id"], original_id.as_str());
    assert!(reply["referenced_message"].is_null());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reactions_toggle_and_respect_rights(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let msg = post(&app, &alex, general, "Abstimmung").await;
    let id = s(&msg["id"]).to_owned();
    assert_eq!(msg["reactions"], json!([]));
    let path = |emoji: &str| format!("/messages/{id}/reactions/{}/@me", enc(emoji));

    for token in [&bob, &bob, &alex] {
        let (status, body) = app.call("PUT", &path("👍"), Some(token), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    assert_eq!(app.call("PUT", &path("🎉"), Some(&bob), None).await.0, StatusCode::NO_CONTENT);

    let seen_by_bob = find(&app, &bob, general, &id).await;
    assert_eq!(
        seen_by_bob["reactions"],
        json!([
            { "emoji": "👍", "count": 2, "me": true, "emoji_id": null, "animated": false },
            { "emoji": "🎉", "count": 1, "me": true, "emoji_id": null, "animated": false }
        ])
    );
    let seen_by_alex = find(&app, &alex, general, &id).await;
    assert_eq!(
        seen_by_alex["reactions"][1],
        json!({ "emoji": "🎉", "count": 1, "me": false, "emoji_id": null, "animated": false })
    );

    // Kein Emoji → invalid_emoji.
    for bad in ["hallo", "a", "<b>"] {
        let (status, body) = app.call("PUT", &path(bad), Some(&bob), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(body["error"], "invalid_emoji");
    }

    // Entfernen (idempotent).
    for _ in 0..2 {
        assert_eq!(app.call("DELETE", &path("🎉"), Some(&bob), None).await.0, StatusCode::NO_CONTENT);
    }
    let seen_by_bob = find(&app, &bob, general, &id).await;
    assert_eq!(
        seen_by_bob["reactions"],
        json!([{ "emoji": "👍", "count": 2, "me": true, "emoji_id": null, "animated": false }])
    );

    // Ohne ADD_REACTIONS im Kanal: 403; Entfernen der eigenen Reaktion geht weiter.
    set_override(&app, &alex, general, &community.default_role, 0, ADD_REACTIONS).await;
    let (status, body) = app.call("PUT", &path("❤️"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(app.call("DELETE", &path("👍"), Some(&bob), None).await.0, StatusCode::NO_CONTENT);

    // Unsichtbarer Kanal: 404.
    set_override(&app, &alex, general, &community.default_role, 0, VIEW_CHANNEL).await;
    assert_eq!(app.call("PUT", &path("👍"), Some(&bob), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.call("DELETE", &path("👍"), Some(&bob), None).await.0, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call(
            "PUT",
            &format!("/messages/{}/reactions/{}/@me", uuid::Uuid::now_v7(), enc("👍")),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reactions_are_limited_per_message(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _, community) = setup(&app).await;
    let msg = post(&app, &alex, &community.text_channel, "Viele Emojis").await;
    let id = s(&msg["id"]).to_owned();
    let emojis: Vec<String> =
        (0x1F600..0x1F600 + 21).map(|c| char::from_u32(c).unwrap().to_string()).collect();
    for emoji in &emojis[..20] {
        let (status, body) =
            app.call("PUT", &format!("/messages/{id}/reactions/{}/@me", enc(emoji)), Some(&alex), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{emoji}: {body}");
    }
    let (status, body) = app
        .call("PUT", &format!("/messages/{id}/reactions/{}/@me", enc(&emojis[20])), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "too_many_reactions");
    // Vorhandenes Emoji geht weiterhin.
    let (status, _) = app
        .call("PUT", &format!("/messages/{id}/reactions/{}/@me", enc(&emojis[0])), Some(&alex), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn mentions_resolve_members_only(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let bob_id = app.user_id(&bob).await;
    let alex_id = app.user_id(&alex).await;

    // carol ist nur Mitglied einer anderen Community.
    let other = app.community(&alex, "Andere").await;
    app.invited_user(&alex, &other, "carol").await;

    let msg =
        post(&app, &alex, general, "Hi @Bob, @carol und @niemand – `@alex` zählt nicht. mail@bob").await;
    assert_eq!(msg["mentions"], json!([bob_id]));
    assert_eq!(msg["mention_everyone"], false);

    // @everyone: Besitzer darf, Bob (Standardrolle) nicht – die Nachricht geht trotzdem raus.
    let everyone = post(&app, &alex, general, "@everyone Treffen!").await;
    assert_eq!(everyone["mention_everyone"], true);
    let attempt = post(&app, &bob, general, "@everyone hallo?").await;
    assert_eq!(attempt["mention_everyone"], false);

    // Mit MENTION_EVERYONE per Kanal-Override darf Bob es auch.
    set_override(&app, &alex, general, &community.default_role, MENTION_EVERYONE, 0).await;
    let allowed = post(&app, &bob, general, "@everyone jetzt aber @alex.").await;
    assert_eq!(allowed["mention_everyone"], true);
    assert_eq!(allowed["mentions"], json!([alex_id]));

    // Ungelesene Erwähnungen für Bob: die Nachricht an @Bob und @everyone von alex (eigene zählen nicht).
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    assert_eq!(detail["mention_counts"][general], 2, "{detail}");
    // Lesestand bis zur neuesten Nachricht → keine ungelesenen Erwähnungen mehr.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/channels/{general}/read"),
            Some(&bob),
            Some(json!({ "message_id": allowed["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&bob), None).await;
    assert!(detail["mention_counts"].get(general).is_none(), "{detail}");

    // Bearbeiten berechnet die Erwähnungen neu.
    let (status, edited) = app
        .call(
            "PATCH",
            &format!("/messages/{}", s(&msg["id"])),
            Some(&alex),
            Some(json!({ "content": "Hi zusammen" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edited["mentions"], json!([]));
}

/// Bearbeiten ist Schreiben: Ohne `SEND_MESSAGES` (z. B. per Override entzogen) keine Änderungen mehr.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn editing_requires_send_messages(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let msg = post(&app, &bob, general, "vorher").await;
    let path = format!("/messages/{}", s(&msg["id"]));
    let edit =
        async |content: &str| app.call("PATCH", &path, Some(&bob), Some(json!({ "content": content }))).await;
    assert_eq!(edit("nachher").await.0, StatusCode::OK);

    set_override(&app, &alex, general, &community.default_role, 0, 1 << 1).await;
    let (status, _) = edit("gesperrt").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(find(&app, &bob, general, s(&msg["id"])).await["content"], "nachher");
}

/// Zählung ungelesener Erwähnungen je Kanal: `@everyone` + direkte Erwähnung zählt einmal, eigene und
/// gelöschte Nachrichten zählen nicht, der Lesestand ist die Grenze.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn mention_counts_per_channel(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let (_, second) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "zweiter", "kind": "text" })),
        )
        .await;
    let second = s(&second["id"]).to_owned();
    let counts = |detail: &Value| detail["mention_counts"].clone();
    let detail_of = |token: &str| {
        let path = format!("/communities/{}", community.id);
        let token = token.to_owned();
        let app = &app;
        async move { app.call("GET", &path, Some(&token), None).await.1 }
    };

    let first = post(&app, &alex, general, "Hallo @bob").await;
    let both = post(&app, &alex, general, "@everyone und besonders @bob").await;
    assert_eq!(both["mention_everyone"], true);
    let everyone = post(&app, &alex, general, "@everyone").await;
    post(&app, &alex, &second, "Drüben: @bob").await;
    // Eigene Nachrichten zählen nicht, auch nicht mit Selbsterwähnung.
    post(&app, &bob, general, "Ich bin @bob").await;
    post(&app, &alex, general, "Ohne Erwähnung").await;

    assert_eq!(counts(&detail_of(&bob).await), json!({ general.as_str(): 3, second.as_str(): 1 }));
    // Alex wird nirgends erwähnt (eigenes @everyone zählt nicht).
    assert!(detail_of(&alex).await["mention_counts"].as_object().unwrap().is_empty());

    // Lesestand nach der ersten Nachricht → nur noch die beiden @everyone.
    let (status, _) = app
        .call(
            "PUT",
            &format!("/channels/{general}/read"),
            Some(&bob),
            Some(json!({ "message_id": first["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(counts(&detail_of(&bob).await)[general], 2);

    // Gelöschte Nachrichten zählen nicht mehr.
    let (status, _) =
        app.call("DELETE", &format!("/messages/{}", s(&everyone["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(counts(&detail_of(&bob).await), json!({ general.as_str(): 1, second.as_str(): 1 }));

    // Kanal unsichtbar → fehlt in der Antwort.
    set_override(&app, &alex, &second, &community.default_role, 0, 1).await;
    assert_eq!(counts(&detail_of(&bob).await), json!({ general.as_str(): 1 }));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn pins_require_manage_messages(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let msg = post(&app, &bob, general, "Wichtig").await;
    let id = s(&msg["id"]).to_owned();
    assert!(msg["pinned_at"].is_null());
    let pin = format!("/channels/{general}/pins/{id}");

    // Bob hat kein MANAGE_MESSAGES.
    assert_eq!(app.call("PUT", &pin, Some(&bob), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.call("DELETE", &pin, Some(&bob), None).await.0, StatusCode::FORBIDDEN);

    // Alex pinnt an (zweimal = idempotent, nur ein Audit-Eintrag).
    for _ in 0..2 {
        assert_eq!(app.call("PUT", &pin, Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    }
    let (status, pins) = app.call("GET", &format!("/channels/{general}/pins"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pins.as_array().unwrap().len(), 1);
    assert_eq!(pins[0]["id"], id.as_str());
    assert!(!pins[0]["pinned_at"].is_null());
    assert!(!find(&app, &bob, general, &id).await["pinned_at"].is_null());

    // Nachricht aus einem anderen Kanal: 404.
    let other = second_text_channel(&app, &alex, &community, "anderer").await;
    assert_eq!(
        app.call("PUT", &format!("/channels/{other}/pins/{id}"), Some(&alex), None).await.0,
        StatusCode::NOT_FOUND
    );

    // Mit MANAGE_MESSAGES per Override darf Bob lösen.
    set_override(&app, &alex, general, &community.default_role, MANAGE_MESSAGES, 0).await;
    assert_eq!(app.call("DELETE", &pin, Some(&bob), None).await.0, StatusCode::NO_CONTENT);
    let (_, pins) = app.call("GET", &format!("/channels/{general}/pins"), Some(&bob), None).await;
    assert_eq!(pins, json!([]));

    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| s(&e["action"])).collect();
    assert_eq!(actions.iter().filter(|a| **a == "message.pin").count(), 1);
    assert_eq!(actions.iter().filter(|a| **a == "message.unpin").count(), 1);
    let entry = log.as_array().unwrap().iter().find(|e| e["action"] == "message.pin").unwrap();
    assert_eq!(entry["target_id"], id.as_str());
    assert_eq!(entry["data"]["channel_id"], general.as_str());

    // Unsichtbarer Kanal: 404, auch für die Pin-Liste.
    set_override(&app, &alex, general, &community.default_role, 0, VIEW_CHANNEL).await;
    assert_eq!(
        app.call("GET", &format!("/channels/{general}/pins"), Some(&bob), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.call("PUT", &pin, Some(&bob), None).await.0, StatusCode::NOT_FOUND);
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Nächstes Event eines bestimmten Typs (max. 5 s); andere werden übersprungen.
async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    let fut = async {
        loop {
            if let Some(Ok(tungstenite::Message::Text(t))) = ws.next().await {
                let v: Value = serde_json::from_str(&t).unwrap();
                if v["t"] == kind {
                    return v["d"].clone();
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("kein {kind} binnen 5 s"))
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gateway_reports_reactions_and_pins(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let general = &community.text_channel;
    let bob_id = app.user_id(&bob).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/gateway")).await.unwrap();
    ws.send(tungstenite::Message::Text(json!({ "op": "identify", "token": alex }).to_string().into()))
        .await
        .unwrap();
    next_of(&mut ws, "READY").await;

    let msg = post(&app, &alex, general, "Hallo @bob").await;
    let id = s(&msg["id"]).to_owned();
    let created = next_of(&mut ws, "MESSAGE_CREATE").await;
    assert_eq!(created["mentions"], json!([bob_id]));

    let path = format!("/messages/{id}/reactions/{}/@me", enc("👋"));
    app.call("PUT", &path, Some(&bob), None).await;
    let added = next_of(&mut ws, "MESSAGE_REACTION_ADD").await;
    assert_eq!(
        added,
        json!({
            "message_id": id, "channel_id": general, "community_id": community.id, "user_id": bob_id,
            "emoji": "👋", "emoji_id": null, "animated": false
        })
    );
    app.call("DELETE", &path, Some(&bob), None).await;
    let removed = next_of(&mut ws, "MESSAGE_REACTION_REMOVE").await;
    assert_eq!(removed["emoji"], "👋");

    app.call("PUT", &format!("/channels/{general}/pins/{id}"), Some(&alex), None).await;
    let updated = next_of(&mut ws, "MESSAGE_UPDATE").await;
    assert_eq!(updated["id"], id.as_str());
    assert!(!updated["pinned_at"].is_null());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn migration_adds_reactions_to_default_roles(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let role_id: uuid::Uuid = community.default_role.parse().unwrap();
    let perms = || async {
        sqlx::query_scalar::<_, i64>("SELECT permissions FROM roles WHERE id = $1")
            .bind(role_id)
            .fetch_one(&db)
            .await
            .unwrap()
    };
    assert_eq!(perms().await & (ADD_REACTIONS | MENTION_EVERYONE), ADD_REACTIONS);

    sqlx::query("UPDATE roles SET permissions = permissions & ~$1::BIGINT")
        .bind(ADD_REACTIONS)
        .execute(&db)
        .await
        .unwrap();
    assert!(lykos_server::MIGRATOR.iter().any(|m| m.version == 8), "Migration 0008 ist registriert");
    sqlx::raw_sql(include_str!("../migrations/0008_message_features.sql")).execute(&db).await.unwrap();
    assert_eq!(perms().await & (ADD_REACTIONS | MENTION_EVERYONE), ADD_REACTIONS);
}
