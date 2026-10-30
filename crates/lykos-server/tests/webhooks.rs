//! Eingehende Webhooks: Verwaltung (Rechte, Audit-Log), Ausführen ohne Anmeldung (Discord-Payload, Embeds,
//! `wait`, Token, Rate-Limit, `allowed_mentions`), Bearbeiten/Löschen per Token und durch Moderatoren.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use serde_json::{Value, json};
use sqlx::PgPool;

const MANAGE_WEBHOOKS: i64 = 1 << 18;
const MANAGE_MESSAGES: i64 = 1 << 2;

async fn setup(app: &TestApp) -> (String, String, TestCommunity) {
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    (alex, bob, community)
}

/// Legt einen Webhook an und gibt (id, token, url) zurück.
async fn create(app: &TestApp, token: &str, channel: &str, name: &str) -> (String, String, String) {
    let (status, body) = app
        .call("POST", &format!("/channels/{channel}/webhooks"), Some(token), Some(json!({ "name": name })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (s(&body["id"]).to_owned(), s(&body["token"]).to_owned(), s(&body["url"]).to_owned())
}

async fn execute(app: &TestApp, id: &str, token: &str, query: &str, body: Value) -> (StatusCode, Value) {
    app.call("POST", &format!("/webhooks/{id}/{token}{query}"), None, Some(body)).await
}

async fn grant(app: &TestApp, owner: &str, community: &TestCommunity, user: &str, perms: i64) {
    let (status, role) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(owner),
            Some(json!({ "name": "Rolle", "permissions": perms })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let user_id = app.user_id(user).await;
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{}/members/{user_id}/roles/{}", community.id, s(&role["id"])),
            Some(owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn management_requires_manage_webhooks(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let channel = &community.text_channel;

    // Ohne Recht: weder anlegen noch auflisten.
    let (status, _) = app
        .call("POST", &format!("/channels/{channel}/webhooks"), Some(&bob), Some(json!({ "name": "x" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app.call("GET", &format!("/channels/{channel}/webhooks"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Sprachkanäle haben keine Webhooks.
    let (status, _) = app
        .call(
            "POST",
            &format!("/channels/{}/webhooks", community.voice_channel),
            Some(&alex),
            Some(json!({ "name": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Besitzer legt an: Token mit Präfix und vollständige URL – nur hier im Klartext.
    let (id, token, url) = create(&app, &alex, channel, "Gitea").await;
    assert!(token.starts_with("lykw_"));
    assert!(url.ends_with(&format!("/api/webhooks/{id}/{token}")));
    let (_, list) = app.call("GET", &format!("/channels/{channel}/webhooks"), Some(&alex), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("token").is_none());
    assert_eq!(list[0]["name"], "Gitea");
    let (_, all) =
        app.call("GET", &format!("/communities/{}/webhooks", community.id), Some(&alex), None).await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    let (_, none) =
        app.call("GET", &format!("/communities/{}/webhooks", community.id), Some(&bob), None).await;
    assert_eq!(none, json!([]));

    // Mit Recht darf Bob umbenennen und Token neu erzeugen.
    grant(&app, &alex, &community, &bob, MANAGE_WEBHOOKS).await;
    let (status, renamed) = app
        .call("PATCH", &format!("/webhooks/{id}"), Some(&bob), Some(json!({ "name": "  Grafana " })))
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Grafana");
    let (status, fresh) = app.call("POST", &format!("/webhooks/{id}/token"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    let new_token = s(&fresh["token"]);
    assert_ne!(new_token, token);
    // Das alte Token gilt nicht mehr.
    let (status, _) = execute(&app, &id, &token, "", json!({ "content": "alt" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = execute(&app, &id, new_token, "", json!({ "content": "neu" })).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Kanal wechseln: nur Textkanäle derselben Community.
    let (_, second) = app
        .call(
            "POST",
            &format!("/communities/{}/channels", community.id),
            Some(&alex),
            Some(json!({ "name": "zwei", "kind": "text" })),
        )
        .await;
    let (status, moved) = app
        .call("PATCH", &format!("/webhooks/{id}"), Some(&bob), Some(json!({ "channel_id": second["id"] })))
        .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["channel_id"], second["id"]);
    let (status, _) = app
        .call(
            "PATCH",
            &format!("/webhooks/{id}"),
            Some(&bob),
            Some(json!({ "channel_id": community.voice_channel })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Audit-Log enthält alle Schritte, aber nie das Token.
    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| s(&e["action"])).collect();
    for action in ["webhook.create", "webhook.update", "webhook.token_reset"] {
        assert!(actions.contains(&action), "{action} fehlt in {actions:?}");
    }
    assert!(!log.to_string().contains("lykw_"));

    // Löschen.
    let (status, _) = app.call("DELETE", &format!("/webhooks/{id}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = execute(&app, &id, new_token, "", json!({ "content": "weg" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn discord_payload_with_embeds(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, community) = setup(&app).await;
    let (id, token, _) = create(&app, &alex, &community.text_channel, "CI").await;

    // Typischer Gitea-/Grafana-Payload.
    let payload = json!({
        "username": "Gitea",
        "avatar_url": "https://tracker.example/avatar.png",
        "content": "Neuer Push",
        "embeds": [{
            "title": "[Lykos] 2 neue Commits",
            "url": "https://git.example.org/lykos/commits",
            "description": "abc123 fix: Tippfehler",
            "color": 5814783,
            "author": { "name": "alex", "icon_url": "https://tracker.example/a.png" },
            "fields": [{ "name": "Branch", "value": "main", "inline": true }],
            "footer": { "text": "Gitea" },
            "image": { "url": "https://tracker.example/i.png" },
            "timestamp": "2026-10-30T12:00:00Z"
        }]
    });
    let (status, message) = execute(&app, &id, &token, "?wait=true", payload).await;
    assert_eq!(status, StatusCode::OK, "{message}");
    assert_eq!(message["author"], Value::Null);
    assert_eq!(message["webhook_id"], id.as_str());
    assert_eq!(message["webhook"]["name"], "Gitea");
    // Fremde Avatare und Bilder werden nie übernommen.
    assert_eq!(message["webhook"]["avatar_url"], Value::Null);
    assert!(!message.to_string().contains("tracker.example"));
    assert_eq!(message["embeds"][0]["title"], "[Lykos] 2 neue Commits");
    assert_eq!(message["embeds"][0]["fields"][0]["name"], "Branch");
    assert_eq!(message["embeds"][0]["color"], 5814783);

    // Ohne wait: 204; die Nachricht steht trotzdem im Verlauf (Name = Webhook-Name).
    let (status, body) = execute(&app, &id, &token, "", json!({ "content": "ohne Name" })).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, list) =
        app.call("GET", &format!("/channels/{}/messages", community.text_channel), Some(&alex), None).await;
    assert_eq!(list[0]["content"], "ohne Name");
    assert_eq!(list[0]["webhook"]["name"], "CI");

    // Leere Nachricht, zu viele Embeds, falsches JSON.
    let (status, _) = execute(&app, &id, &token, "", json!({ "embeds": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) =
        execute(&app, &id, &token, "", json!({ "embeds": vec![json!({ "title": "x" }); 11] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Multipart mit payload_json geht, Dateien nicht.
    let boundary = "lykosgrenze";
    let form = |extra: &str| {
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n\r\n{{\"content\":\"aus Formular\"}}\r\n{extra}--{boundary}--\r\n"
        )
    };
    let req = |body: String| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/webhooks/{id}/{token}?wait=true"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap()
    };
    let (status, message) = app.send(req(form(""))).await;
    assert_eq!(status, StatusCode::OK, "{message}");
    assert_eq!(message["content"], "aus Formular");
    let file = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhallo\r\n"
    );
    let (status, body) = app.send(req(form(&file))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "attachments_not_supported");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn wrong_token_and_rate_limit(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, _bob, community) = setup(&app).await;
    let (id, token, _) = create(&app, &alex, &community.text_channel, "Kuma").await;

    let (status, body) = execute(&app, &id, "lykw_falsch", "", json!({ "content": "x" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid_webhook_token");
    let unknown = uuid::Uuid::now_v7();
    let (status, _) = execute(&app, &unknown.to_string(), &token, "", json!({ "content": "x" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Info per Token verrät den Ersteller nicht.
    let (status, info) = app.call("GET", &format!("/webhooks/{id}/{token}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["name"], "Kuma");
    assert_eq!(info["created_by"], Value::Null);

    // 30 Aufrufe pro Minute (der Info-Aufruf zählt mit).
    let mut limited = None;
    for i in 0..40 {
        let (status, _) = execute(&app, &id, &token, "", json!({ "content": format!("ping {i}") })).await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(i);
            break;
        }
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert_eq!(limited, Some(29));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn allowed_mentions_default_blocks_everyone(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let (id, token, _) = create(&app, &alex, &community.text_channel, "Alarm").await;
    let bob_id = app.user_id(&bob).await;

    // Standard: Nutzer ja, @everyone nein.
    let (_, message) =
        execute(&app, &id, &token, "?wait=true", json!({ "content": "@everyone @bob Alarm" })).await;
    assert_eq!(message["mention_everyone"], false);
    assert_eq!(message["mentions"], json!([bob_id]));

    // parse: [] → niemand.
    let (_, message) = execute(
        &app,
        &id,
        &token,
        "?wait=true",
        json!({ "content": "@everyone @bob", "allowed_mentions": { "parse": [] } }),
    )
    .await;
    assert_eq!(message["mention_everyone"], false);
    assert_eq!(message["mentions"], json!([]));

    // Ausdrücklich erlaubt: @everyone wirkt.
    let (_, message) = execute(
        &app,
        &id,
        &token,
        "?wait=true",
        json!({ "content": "@everyone", "allowed_mentions": { "parse": ["everyone"] } }),
    )
    .await;
    assert_eq!(message["mention_everyone"], true);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn edit_and_delete_webhook_messages(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, community) = setup(&app).await;
    let channel = &community.text_channel;
    let (id, token, _) = create(&app, &alex, channel, "Bot").await;
    let (_, message) = execute(&app, &id, &token, "?wait=true", json!({ "content": "Version 1" })).await;
    let mid = s(&message["id"]).to_owned();

    // Bearbeiten per Token.
    let (status, edited) = app
        .call(
            "PATCH",
            &format!("/webhooks/{id}/{token}/messages/{mid}"),
            None,
            Some(json!({ "content": "Version 2", "embeds": [{ "title": "Status", "description": "grün" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["content"], "Version 2");
    assert_eq!(edited["embeds"][0]["title"], "Status");
    assert!(edited["edited_at"].is_string());

    // Fremde Nachrichten kann der Webhook nicht anfassen.
    let (_, human) = app
        .call("POST", &format!("/channels/{channel}/messages"), Some(&alex), Some(json!({ "content": "hi" })))
        .await;
    let (status, _) = app
        .call(
            "PATCH",
            &format!("/webhooks/{id}/{token}/messages/{}", s(&human["id"])),
            None,
            Some(json!({ "content": "gehackt" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Menschen können Webhook-Nachrichten nicht bearbeiten.
    let (status, _) =
        app.call("PATCH", &format!("/messages/{mid}"), Some(&alex), Some(json!({ "content": "x" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Antworten, Reaktionen und Pins funktionieren auf Webhook-Nachrichten.
    let (status, reply) = app
        .call(
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&bob),
            Some(json!({ "content": "danke", "reply_to_id": mid })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");
    assert_eq!(reply["referenced_message"]["webhook_name"], "Bot");
    assert_eq!(reply["referenced_message"]["author"], Value::Null);
    let (status, _) =
        app.call("PUT", &format!("/messages/{mid}/reactions/%F0%9F%91%8D/@me"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("PUT", &format!("/channels/{channel}/pins/{mid}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, found) =
        app.call("GET", &format!("/communities/{}/search?q=Version", community.id), Some(&alex), None).await;
    assert_eq!(found.as_array().map(Vec::len), Some(1), "{found}");

    // Löschen per Token.
    let (status, _) = app.call("DELETE", &format!("/webhooks/{id}/{token}/messages/{mid}"), None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.call("DELETE", &format!("/webhooks/{id}/{token}/messages/{mid}"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Moderatoren löschen Webhook-Nachrichten wie andere; einfache Mitglieder nicht.
    let (_, second) = execute(&app, &id, &token, "?wait=true", json!({ "content": "noch eine" })).await;
    let second = s(&second["id"]).to_owned();
    let (status, _) = app.call("DELETE", &format!("/messages/{second}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    grant(&app, &alex, &community, &bob, MANAGE_MESSAGES).await;
    let (status, _) = app.call("DELETE", &format!("/messages/{second}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Gelöschter Webhook: Nachrichten bleiben mit Namen.
    let (_, third) = execute(&app, &id, &token, "?wait=true", json!({ "content": "bleibt" })).await;
    let (status, _) = app.call("DELETE", &format!("/webhooks/{id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = app.call("GET", &format!("/channels/{channel}/messages"), Some(&alex), None).await;
    let kept = list.as_array().unwrap().iter().find(|m| m["id"] == third["id"]).unwrap();
    assert_eq!(kept["webhook"]["name"], "Bot");
    assert_eq!(kept["webhook_id"], Value::Null);
}
