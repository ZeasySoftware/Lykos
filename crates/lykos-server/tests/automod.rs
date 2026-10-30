//! Integrationstests für den AutoMod: Verwaltung (Rechte, Prüfung), Blockieren beim Erstellen und Bearbeiten,
//! Ausnahmen (Rolle, Kanal, Verwalter), Alarm im Protokollkanal, Audit-Log, Erwähnungs- und Flut-Regeln sowie
//! Nachrichten von Bots und Webhooks.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use serde_json::{Value, json};
use sqlx::PgPool;

const MANAGE_COMMUNITY: i64 = 1 << 11;
const ADMINISTRATOR: i64 = 1 << 20;

struct Fixture {
    app: TestApp,
    owner: String,
    member: String,
    community: TestCommunity,
}

impl Fixture {
    async fn new(db: PgPool) -> Self {
        let app = TestApp::new(db);
        let owner = app.user("alice", None).await;
        let community = app.community(&owner, "Rudel").await;
        let member = app.invited_user(&owner, &community, "bob").await;
        Self { app, owner, member, community }
    }

    async fn put_rules(&self, token: &str, rules: Value) -> (StatusCode, Value) {
        self.app
            .call(
                "PUT",
                &format!("/communities/{}/automod", self.community.id),
                Some(token),
                Some(json!({ "rules": rules })),
            )
            .await
    }

    /// Regeln als Besitzer setzen (erwartet Erfolg).
    async fn set_rules(&self, rules: Value) -> Value {
        let (status, body) = self.put_rules(&self.owner, rules).await;
        assert_eq!(status, StatusCode::OK, "Regeln speichern: {body}");
        body
    }

    async fn send(&self, token: &str, channel: &str, content: &str) -> (StatusCode, Value) {
        self.app
            .call(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(token),
                Some(json!({ "content": content })),
            )
            .await
    }

    /// Nachrichten eines Kanals ohne Beitrittsmeldungen (`#allgemein` ist Systemkanal).
    async fn messages(&self, channel: &str) -> Vec<Value> {
        let (status, body) =
            self.app.call("GET", &format!("/channels/{channel}/messages"), Some(&self.owner), None).await;
        assert_eq!(status, StatusCode::OK);
        body.as_array().unwrap().iter().filter(|m| m["kind"] != "member_join").cloned().collect()
    }

    async fn channel(&self, name: &str) -> String {
        let (status, ch) = self
            .app
            .call(
                "POST",
                &format!("/communities/{}/channels", self.community.id),
                Some(&self.owner),
                Some(json!({ "name": name, "kind": "text" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{ch}");
        s(&ch["id"]).to_owned()
    }

    /// Legt eine Rolle an und gibt sie `user` (Token).
    async fn role_for(&self, user: &str, name: &str, permissions: i64) -> String {
        let (status, role) = self
            .app
            .call(
                "POST",
                &format!("/communities/{}/roles", self.community.id),
                Some(&self.owner),
                Some(json!({ "name": name, "permissions": permissions })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{role}");
        let role_id = s(&role["id"]).to_owned();
        let user_id = self.app.user_id(user).await;
        let (status, _) = self
            .app
            .call(
                "PUT",
                &format!("/communities/{}/members/{user_id}/roles/{role_id}", self.community.id),
                Some(&self.owner),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        role_id
    }
}

fn keyword_rule(terms: &[&str]) -> Value {
    json!({ "kind": "keyword", "enabled": true, "config": { "terms": terms } })
}

#[sqlx::test]
async fn management_requires_manage_community(db: PgPool) {
    let f = Fixture::new(db).await;
    let path = format!("/communities/{}/automod", f.community.id);

    // Voreinstellung: alle vier Regeln aus, Aktion „blockieren“.
    let (status, rules) = f.app.call("GET", &path, Some(&f.owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let kinds: Vec<&str> = rules.as_array().unwrap().iter().map(|r| s(&r["kind"])).collect();
    assert_eq!(kinds, ["keyword", "links", "mentions", "spam"]);
    assert!(rules.as_array().unwrap().iter().all(|r| r["enabled"] == false && r["actions"]["block"] == true));
    assert_eq!(rules[2]["config"]["max_mentions"], 5);

    // Normale Mitglieder dürfen weder lesen noch ändern.
    let (status, _) = f.app.call("GET", &path, Some(&f.member), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.put_rules(&f.member, json!([keyword_rule(&["foo"])])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Fremde sehen die Community gar nicht.
    let other = f.app.community(&f.owner, "Andere").await;
    let stranger = f.app.invited_user(&f.owner, &other, "mallory").await;
    let (status, _) = f.app.call("GET", &path, Some(&stranger), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Mit MANAGE_COMMUNITY geht es.
    f.role_for(&f.member, "Mods", MANAGE_COMMUNITY).await;
    let (status, rules) = f.put_rules(&f.member, json!([keyword_rule(&["  Foo  ", "foo", "*bar*"])])).await;
    assert_eq!(status, StatusCode::OK, "{rules}");
    assert_eq!(rules[0]["enabled"], true);
    // Duplikate (Groß/Klein) entfernt, Leerraum bereinigt.
    assert_eq!(rules[0]["config"]["terms"], json!(["Foo", "*bar*"]));

    // Änderung steht im Audit-Log.
    let (_, log) =
        f.app.call("GET", &format!("/communities/{}/audit-log", f.community.id), Some(&f.owner), None).await;
    assert!(log.as_array().unwrap().iter().any(|e| e["action"] == "automod.update"));
}

#[sqlx::test]
async fn management_validates_input(db: PgPool) {
    let f = Fixture::new(db).await;
    let cases = [
        (keyword_rule(&["*"]), "invalid_keyword"),
        (keyword_rule(&[&"x".repeat(61)]), "invalid_keyword"),
        (
            json!({ "kind": "links", "enabled": true, "config": { "allowed_domains": ["kein host"] } }),
            "invalid_domain",
        ),
        (json!({ "kind": "mentions", "enabled": true, "config": { "max_mentions": 0 } }), "invalid_limit"),
        (json!({ "kind": "spam", "enabled": true, "config": { "max_messages": 1 } }), "invalid_limit"),
        (
            json!({ "kind": "mentions", "enabled": true, "actions": { "block": false, "alert": true } }),
            "alert_channel_required",
        ),
        (
            json!({ "kind": "mentions", "enabled": true, "actions": { "block": false, "alert": false, "audit": false } }),
            "invalid_actions",
        ),
        (json!({ "kind": "mentions", "alert_channel_id": f.community.voice_channel }), "invalid_channel"),
        (json!({ "kind": "mentions", "custom_message": "x".repeat(151) }), "invalid_message"),
    ];
    for (rule, code) in cases {
        let (status, body) = f.put_rules(&f.owner, json!([rule])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rule}");
        assert_eq!(body["error"], code, "{rule}");
    }
    // Zu viele Begriffe.
    let many: Vec<String> = (0..1001).map(|i| format!("wort{i}")).collect();
    let (status, _) =
        f.put_rules(&f.owner, json!([{ "kind": "keyword", "config": { "terms": many } }])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Protokollkanal einer anderen Community.
    let other = f.app.community(&f.owner, "Andere").await;
    let (_, body) =
        f.put_rules(&f.owner, json!([{ "kind": "links", "alert_channel_id": other.text_channel }])).await;
    assert_eq!(body["error"], "invalid_channel");
    // Doppelte Art in einer Anfrage.
    let (status, _) = f.put_rules(&f.owner, json!([{ "kind": "links" }, { "kind": "links" }])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn keyword_blocks_create_and_edit(db: PgPool) {
    let f = Fixture::new(db).await;
    let channel = f.community.text_channel.clone();
    let (status, sent) = f.send(&f.member, &channel, "Hallo zusammen").await;
    assert_eq!(status, StatusCode::CREATED);

    f.set_rules(json!([{
        "kind": "keyword",
        "enabled": true,
        "custom_message": "Bitte freundlich bleiben.",
        "config": { "terms": ["spam*", "böse"], "allow": [] },
    }]))
    .await;

    // Blockiert: verständlicher Fehler, nichts gespeichert.
    let (status, body) = f.send(&f.member, &channel, "Hier gibt's SPAMMER").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "automod_blocked");
    assert_eq!(body["rule"], "keyword");
    assert_eq!(body["rule_name"], "Wortfilter");
    assert_eq!(body["message"], "Bitte freundlich bleiben.");
    // Unicode: Vollbreite-Großbuchstaben.
    let (status, _) = f.send(&f.member, &channel, "ＢÖＳＥ!").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let contents: Vec<String> =
        f.messages(&channel).await.iter().map(|m| s(&m["content"]).to_owned()).collect();
    assert_eq!(contents, ["Hallo zusammen"]);

    // Erlaubtes geht weiter durch (ganzes Wort „böse“, nicht „Böses“).
    let (status, _) = f.send(&f.member, &channel, "Nichts Böses gemeint").await;
    assert_eq!(status, StatusCode::CREATED);

    // Bearbeiten wird ebenfalls geprüft – die alte Fassung bleibt.
    let id = s(&sent["id"]);
    let (status, body) = f
        .app
        .call("PATCH", &format!("/messages/{id}"), Some(&f.member), Some(json!({ "content": "spam!" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "automod_blocked");
    let stored: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(stored, "Hallo zusammen");

    // Besitzer ist ausgenommen.
    let (status, _) = f.send(&f.owner, &channel, "spam spam spam").await;
    assert_eq!(status, StatusCode::CREATED);

    // Regel aus → Cache wird invalidiert, Nachricht geht durch.
    f.set_rules(json!([{ "kind": "keyword", "enabled": false, "config": { "terms": ["spam*"] } }])).await;
    let (status, _) = f.send(&f.member, &channel, "spam").await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test]
async fn exemptions_for_roles_channels_and_managers(db: PgPool) {
    let f = Fixture::new(db).await;
    let carol = f.app.invited_user(&f.owner, &f.community, "carol").await;
    let dave = f.app.invited_user(&f.owner, &f.community, "dave").await;
    let offtopic = f.channel("offtopic").await;
    let trusted = f.role_for(&f.member, "Vertraut", 0).await;
    f.role_for(&carol, "Admins", ADMINISTRATOR).await;

    f.set_rules(json!([{
        "kind": "links",
        "enabled": true,
        "exempt_role_ids": [trusted],
        "exempt_channel_ids": [offtopic],
        "config": { "allowed_domains": ["example.org"] },
    }]))
    .await;

    let general = f.community.text_channel.clone();
    // Ohne Ausnahme blockiert, erlaubte Domain (inkl. Subdomain) nicht.
    let (status, body) = f.send(&dave, &general, "siehe https://evil.com/x").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["rule"], "links");
    let (status, _) = f.send(&dave, &general, "siehe https://docs.example.org/x").await;
    assert_eq!(status, StatusCode::CREATED);
    // Links zur eigenen Instanz sind immer erlaubt.
    let (status, _) = f.send(&dave, &general, "http://localhost/invite/abc").await;
    assert_eq!(status, StatusCode::CREATED);
    // Kanal-Ausnahme.
    let (status, _) = f.send(&dave, &offtopic, "https://evil.com").await;
    assert_eq!(status, StatusCode::CREATED);
    // Rollen-Ausnahme.
    let (status, _) = f.send(&f.member, &general, "https://evil.com").await;
    assert_eq!(status, StatusCode::CREATED);
    // Administratoren sind immer ausgenommen.
    let (status, _) = f.send(&carol, &general, "https://evil.com").await;
    assert_eq!(status, StatusCode::CREATED);

    // Rollen-/Kanal-IDs fremder Communities werden verworfen.
    let other = f.app.community(&f.owner, "Andere").await;
    let rules = f
        .set_rules(json!([{ "kind": "links", "exempt_role_ids": [other.default_role], "exempt_channel_ids": [other.text_channel] }]))
        .await;
    assert_eq!(rules[1]["exempt_role_ids"], json!([]));
    assert_eq!(rules[1]["exempt_channel_ids"], json!([]));
}

#[sqlx::test]
async fn alert_in_log_channel_and_audit(db: PgPool) {
    let f = Fixture::new(db).await;
    let log = f.channel("mod-log").await;
    let general = f.community.text_channel.clone();

    // Nur melden (nicht blockieren) – mit Alarm und Audit-Eintrag.
    f.set_rules(json!([{
        "kind": "links",
        "enabled": true,
        "actions": { "block": false, "alert": true, "audit": true },
        "alert_channel_id": log,
        "config": { "allowed_domains": [], "block_foreign_invites": true },
    }]))
    .await;
    let (status, _) = f.send(&f.member, &general, "Neu: https://evil.com/angebot").await;
    assert_eq!(status, StatusCode::CREATED);

    let alerts = f.messages(&log).await;
    assert_eq!(alerts.len(), 1);
    let alert = &alerts[0];
    assert_eq!(alert["kind"], "automod_alert");
    assert_eq!(alert["author"]["username"], "bob");
    let data: Value = serde_json::from_str(s(&alert["content"])).unwrap();
    assert_eq!(data["rule"], "links");
    assert_eq!(data["channel_id"], general.as_str());
    assert_eq!(data["blocked"], false);
    assert_eq!(data["detail"], "Link zu evil.com");
    assert_eq!(data["excerpt"], "Neu: https://evil.com/angebot");

    let (_, audit) =
        f.app.call("GET", &format!("/communities/{}/audit-log", f.community.id), Some(&f.owner), None).await;
    let entry =
        audit.as_array().unwrap().iter().find(|e| e["action"] == "automod.flag").expect("Audit-Eintrag");
    assert_eq!(entry["actor"]["username"], "bob");
    assert_eq!(entry["data"]["rule"], "links");

    // Blockieren + Alarm: Alarm vermerkt „blockiert“, die Nachricht fehlt im Kanal.
    f.set_rules(json!([{
        "kind": "links",
        "enabled": true,
        "actions": { "block": true, "alert": true },
        "alert_channel_id": log,
    }]))
    .await;
    let (status, _) = f.send(&f.member, &general, "https://evil.com/2").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let alerts = f.messages(&log).await;
    assert_eq!(alerts.len(), 2);
    let data: Value = serde_json::from_str(s(&alerts[0]["content"])).unwrap();
    assert_eq!(data["blocked"], true);
    assert!(!f.messages(&general).await.iter().any(|m| m["content"] == "https://evil.com/2"));

    // Alarme tauchen nicht in der Suche auf.
    let (_, hits) = f
        .app
        .call("GET", &format!("/communities/{}/search?q=evil", f.community.id), Some(&f.owner), None)
        .await;
    assert!(hits.as_array().unwrap().iter().all(|m| m["kind"] != "automod_alert"));

    // Systemnachrichten lassen sich nicht bearbeiten.
    let (status, _) = f
        .app
        .call(
            "PATCH",
            &format!("/messages/{}", s(&alerts[0]["id"])),
            Some(&f.member),
            Some(json!({ "content": "x" })),
        )
        .await;
    assert_ne!(status, StatusCode::OK);

    // Gelöschter Protokollkanal: Regel greift weiter, nur der Alarm entfällt.
    let (status, _) = f.app.call("DELETE", &format!("/channels/{log}"), Some(&f.owner), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = f.send(&f.member, &general, "https://evil.com/3").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn mention_limit(db: PgPool) {
    let f = Fixture::new(db).await;
    for name in ["carol", "dave", "erin"] {
        f.app.invited_user(&f.owner, &f.community, name).await;
    }
    f.set_rules(json!([{ "kind": "mentions", "enabled": true, "config": { "max_mentions": 2 } }])).await;
    let general = f.community.text_channel.clone();

    let (status, body) = f.send(&f.member, &general, "@carol @dave @erin kommt ihr?").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["rule"], "mentions");
    // Genau an der Grenze, doppelte Erwähnungen und Unbekannte zählen nicht.
    let (status, _) = f.send(&f.member, &general, "@carol @dave @carol @niemand @keiner").await;
    assert_eq!(status, StatusCode::CREATED);
    // @everyone ist keine Nutzer-Erwähnung.
    let (status, _) = f.send(&f.member, &general, "@everyone @carol @dave").await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test]
async fn spam_rate_and_duplicates(db: PgPool) {
    let f = Fixture::new(db).await;
    let log = f.channel("mod-log").await;
    f.set_rules(json!([{
        "kind": "spam",
        "enabled": true,
        "actions": { "block": true, "alert": true },
        "alert_channel_id": log,
        "config": { "max_messages": 3, "per_seconds": 60, "max_duplicates": 0 },
    }]))
    .await;
    let general = f.community.text_channel.clone();
    for i in 0..3 {
        let (status, _) = f.send(&f.member, &general, &format!("Nachricht {i}")).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, body) = f.send(&f.member, &general, "Nachricht 4").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["rule"], "spam");
    let (status, _) = f.send(&f.member, &general, "Nachricht 5").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nur ein Alarm pro Serie.
    assert_eq!(f.messages(&log).await.len(), 1);
    // Bearbeiten zählt nicht als neue Nachricht.
    let messages = f.messages(&general).await;
    let first = s(&messages.iter().find(|m| m["content"] == "Nachricht 0").unwrap()["id"]).to_owned();
    let (status, _) = f
        .app
        .call("PATCH", &format!("/messages/{first}"), Some(&f.member), Some(json!({ "content": "geändert" })))
        .await;
    assert_eq!(status, StatusCode::OK);
    // Andere Nutzer zählen getrennt.
    let carol = f.app.invited_user(&f.owner, &f.community, "carol").await;
    let (status, _) = f.send(&carol, &general, "Hallo").await;
    assert_eq!(status, StatusCode::CREATED);

    // Wiederholungen: dritte identische Nachricht in Folge blockiert.
    f.set_rules(
        json!([{ "kind": "spam", "enabled": true, "config": { "max_messages": 0, "max_duplicates": 2 } }]),
    )
    .await;
    let dave = f.app.invited_user(&f.owner, &f.community, "dave").await;
    for _ in 0..2 {
        let (status, _) = f.send(&dave, &general, "Kauft Zeug!").await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, _) = f.send(&dave, &general, "  kauft ZEUG!  ").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.send(&dave, &general, "Etwas anderes").await;
    assert_eq!(status, StatusCode::CREATED);
}

/// Anfrage mit `Authorization: Bot <token>`.
async fn bot_call(app: &TestApp, method: &str, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(format!("/api{path}"))
        .header(header::AUTHORIZATION, format!("Bot {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.send(req).await
}

#[sqlx::test]
async fn webhook_messages_are_checked(db: PgPool) {
    let f = Fixture::new(db).await;
    let log = f.channel("mod-log").await;
    let general = f.community.text_channel.clone();
    let (status, hook) = f
        .app
        .call("POST", &format!("/channels/{general}/webhooks"), Some(&f.owner), Some(json!({ "name": "CI" })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let base = format!("/webhooks/{}/{}", s(&hook["id"]), s(&hook["token"]));
    let (status, sent) = f
        .app
        .call("POST", &format!("{base}?wait=true"), None, Some(json!({ "content": "Build grün" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    f.set_rules(json!([{
        "kind": "keyword",
        "enabled": true,
        "actions": { "block": true, "alert": true, "audit": true },
        "alert_channel_id": log,
        "config": { "terms": ["geheim"] },
    }]))
    .await;

    // Senden: blockiert mit derselben Antwort wie für Mitglieder, nichts gespeichert.
    let (status, body) = f
        .app
        .call("POST", &base, None, Some(json!({ "content": "Das Passwort ist geheim", "username": "Bot X" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "automod_blocked");
    assert_eq!(body["rule"], "keyword");
    assert!(!f.messages(&general).await.iter().any(|m| s(&m["content"]).contains("geheim")));

    // Alarm ohne Autor, mit dem Webhook-Namen als Absender; ohne Akteur kein Audit-Eintrag.
    let alerts = f.messages(&log).await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0]["kind"], "automod_alert");
    assert!(alerts[0]["author"].is_null());
    assert_eq!(alerts[0]["webhook"]["name"], "Bot X");
    assert!(alerts[0]["webhook_id"].is_null());
    let (_, audit) =
        f.app.call("GET", &format!("/communities/{}/audit-log", f.community.id), Some(&f.owner), None).await;
    assert!(audit.as_array().unwrap().iter().all(|e| e["action"] != "automod.block"));

    // Bearbeiten per Token wird ebenfalls geprüft – die alte Fassung bleibt.
    let id = s(&sent["id"]);
    let (status, body) = f
        .app
        .call("PATCH", &format!("{base}/messages/{id}"), None, Some(json!({ "content": "geheim!" })))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "automod_blocked");
    let stored: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(stored, "Build grün");
    let edit_alert: Value = serde_json::from_str(s(&f.messages(&log).await[0]["content"])).unwrap();
    assert_eq!(edit_alert["edit"], true);

    // Erlaubtes geht weiter durch.
    let (status, _) = f.app.call("POST", &base, None, Some(json!({ "content": "Build rot" }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = f
        .app
        .call("PATCH", &format!("{base}/messages/{id}"), None, Some(json!({ "content": "Build gelb" })))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn bot_messages_are_checked_like_members(db: PgPool) {
    let f = Fixture::new(db).await;
    let general = f.community.text_channel.clone();
    let (status, bot) =
        f.app.call("POST", "/bots", Some(&f.owner), Some(json!({ "username": "helfer" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{bot}");
    let bot_token = s(&bot["token"]).to_owned();
    let (status, _) = f
        .app
        .call(
            "POST",
            &format!("/communities/{}/bots", f.community.id),
            Some(&f.owner),
            Some(json!({ "bot_id": bot["id"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let path = format!("/channels/{general}/messages");
    let (status, sent) = bot_call(&f.app, "POST", &path, &bot_token, json!({ "content": "Hallo" })).await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");

    f.set_rules(json!([keyword_rule(&["geheim"])])).await;

    // Senden und Bearbeiten: blockiert.
    let (status, body) = bot_call(&f.app, "POST", &path, &bot_token, json!({ "content": "geheim" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "automod_blocked");
    let (status, body) = bot_call(
        &f.app,
        "PATCH",
        &format!("/messages/{}", s(&sent["id"])),
        &bot_token,
        json!({ "content": "geheim" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!f.messages(&general).await.iter().any(|m| m["content"] == "geheim"));

    // Rollen des Bots zählen für Ausnahmen.
    let (status, role) = f
        .app
        .call(
            "POST",
            &format!("/communities/{}/roles", f.community.id),
            Some(&f.owner),
            Some(json!({ "name": "Bots", "permissions": 0 })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id = s(&role["id"]).to_owned();
    let (status, _) = f
        .app
        .call(
            "PUT",
            &format!("/communities/{}/members/{}/roles/{role_id}", f.community.id, s(&bot["id"])),
            Some(&f.owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    f.set_rules(json!([{
        "kind": "keyword",
        "enabled": true,
        "exempt_role_ids": [role_id],
        "config": { "terms": ["geheim"] },
    }]))
    .await;
    let (status, body) = bot_call(&f.app, "POST", &path, &bot_token, json!({ "content": "geheim" })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}
