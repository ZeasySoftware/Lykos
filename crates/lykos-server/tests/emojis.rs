//! Integrationstests für Custom-Emotes (Paket I): Upload samt Prüfungen, Rechte, Limit, Umbenennen/Löschen,
//! Abruf, Verwendung in Nachrichten (fremde Emotes werden degradiert) und Reaktionen.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use http_body_util::BodyExt;
use lykos_server::{
    config::Config,
    storage::{LocalStorage, Storage},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

const BOUNDARY: &str = "lykos-test-boundary-emoji";
const MANAGE_EMOJIS: i64 = 1 << 16;

/// Minimaler PNG-Kopf (Signatur + IHDR) – reicht für Typ-Erkennung und Maße.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0");
    v.extend_from_slice(&[0u8; 64]);
    v
}

/// Minimaler GIF-Kopf (logischer Bildschirm mit Breite/Höhe).
fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend_from_slice(&width.to_le_bytes());
    v.extend_from_slice(&height.to_le_bytes());
    v.extend_from_slice(&[0u8; 64]);
    v
}

struct Emojis {
    app: TestApp,
    dir: TempDir,
}

impl Emojis {
    fn new(db: PgPool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_tests();
        config.storage_dir = dir.path().to_path_buf();
        Self { app: TestApp::with_config(db, config), dir }
    }

    async fn stored_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> =
            LocalStorage::new(self.dir.path()).list().await.unwrap().into_iter().map(|o| o.key).collect();
        keys.sort();
        keys
    }

    async fn upload(&self, token: &str, community: &str, name: &str, data: &[u8]) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n\
             --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"e.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/communities/{community}/emojis"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        self.app.send(req).await
    }

    /// Lädt hoch und erwartet Erfolg; gibt das Emote zurück.
    async fn uploaded(&self, token: &str, community: &str, name: &str) -> Value {
        let (status, emoji) = self.upload(token, community, name, &png(64, 64)).await;
        assert_eq!(status, StatusCode::CREATED, "{emoji}");
        emoji
    }

    async fn get_raw(&self, token: Option<&str>, url: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method("GET").uri(url);
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let res = self.app.router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
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

/// Alex (Instanz-Admin) besitzt „Rudel“ mit Bob; Carol besitzt „Andere“; Dave ist in beiden.
struct Setup {
    alex: String,
    bob: String,
    carol: String,
    dave: String,
    rudel: TestCommunity,
    andere: TestCommunity,
}

async fn setup(app: &TestApp) -> Setup {
    let alex = app.user("alex", None).await;
    let rudel = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &rudel, "bob").await;
    let dave = app.invited_user(&alex, &rudel, "dave").await;
    let lobby = app.community(&alex, "Lobby").await;
    let carol = app.invited_user(&alex, &lobby, "carol").await;
    let andere = app.community(&carol, "Andere").await;
    let (_, invite) =
        app.call("POST", &format!("/communities/{}/invites", andere.id), Some(&carol), Some(json!({}))).await;
    let (status, _) =
        app.call("POST", &format!("/invites/{}/accept", s(&invite["code"])), Some(&dave), None).await;
    assert_eq!(status, StatusCode::OK);
    Setup { alex, bob, carol, dave, rudel, andere }
}

async fn grant(app: &TestApp, owner: &str, community: &TestCommunity, member: &str, perms: i64) {
    let (status, role) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(owner),
            Some(json!({ "name": "Emote-Team", "permissions": perms })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let member_id = app.user_id(member).await;
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{}/members/{member_id}/roles/{}", community.id, s(&role["id"])),
            Some(owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn upload_rights_validation_and_fetch(db: PgPool) {
    let e = Emojis::new(db);
    let t = setup(&e.app).await;
    let rudel = t.rudel.id.as_str();

    // Ohne Recht 403, Fremde 404, ohne Login 401.
    let (status, _) = e.upload(&t.bob, rudel, "wolf", &png(64, 64)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = e.upload(&t.carol, rudel, "wolf", &png(64, 64)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Besitzer darf.
    let emoji = e.uploaded(&t.alex, rudel, "wolf").await;
    let id = s(&emoji["id"]).to_owned();
    assert_eq!(emoji["name"], "wolf");
    assert_eq!(emoji["animated"], false);
    assert_eq!(emoji["community_id"], rudel);
    assert_eq!(emoji["uploader"]["username"], "alex");
    assert_eq!(emoji["url"], format!("/api/emojis/{id}.png"));
    assert_eq!(e.stored_keys().await, vec![id.clone()]);

    // Mit MANAGE_EMOJIS darf auch Bob; GIF wird als animiert erkannt.
    grant(&e.app, &t.alex, &t.rudel, &t.bob, MANAGE_EMOJIS).await;
    let (status, tanz) = e.upload(&t.bob, rudel, "tanz", &gif(32, 32)).await;
    assert_eq!(status, StatusCode::CREATED, "{tanz}");
    assert_eq!(tanz["animated"], true);
    assert!(s(&tanz["url"]).ends_with(".gif"));

    // Ungültige Namen, Typen, Maße, Größe, doppelte Namen.
    for name in ["a", "mit leer", "ümlaut", &"x".repeat(33)] {
        let (status, body) = e.upload(&t.alex, rudel, name, &png(64, 64)).await;
        assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_name"), "{name}");
    }
    for data in
        [b"<svg onload=alert(1)></svg>".to_vec(), png(129, 64), gif(64, 200), b"\xff\xd8\xff\xe0".to_vec()]
    {
        let (status, body) = e.upload(&t.alex, rudel, "bild", &data).await;
        assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_image"));
    }
    let mut big = png(64, 64);
    big.resize(300 * 1024, 0);
    let (status, body) = e.upload(&t.alex, rudel, "riesig", &big).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::PAYLOAD_TOO_LARGE, "file_too_large"));
    let (status, body) = e.upload(&t.alex, rudel, "WOLF", &png(16, 16)).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "emoji_name_taken"));
    assert_eq!(e.stored_keys().await.len(), 2);

    // Listen: Community (Mitglieder), eigene (alle Communities), Fremde 404.
    let (status, list) = e.app.call("GET", &format!("/communities/{rudel}/emojis"), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|x| s(&x["name"])).collect();
    assert_eq!(names, vec!["tanz", "wolf"]);
    let (status, _) = e.app.call("GET", &format!("/communities/{rudel}/emojis"), Some(&t.carol), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, mine) = e.app.call("GET", "/emojis", Some(&t.carol), None).await;
    assert_eq!(mine, json!([]));
    let (_, mine) = e.app.call("GET", "/emojis", Some(&t.dave), None).await;
    assert_eq!(mine.as_array().unwrap().len(), 2);

    // Bild: jeder Angemeldete, cachebar; Endung egal; ohne Login 401; unbekannt 404.
    let (status, headers, bytes) = e.get_raw(Some(&t.carol), &format!("/api/emojis/{id}.png")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(headers[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
    assert_eq!(bytes, png(64, 64));
    let (status, _, _) = e.get_raw(Some(&t.carol), &format!("/api/emojis/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = e.get_raw(None, &format!("/api/emojis/{id}.png")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = e.get_raw(Some(&t.carol), &format!("/api/emojis/{}.png", Uuid::now_v7())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = e.get_raw(Some(&t.carol), "/api/emojis/quatsch.png").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn limit_per_community(db: PgPool) {
    let e = Emojis::new(db);
    let t = setup(&e.app).await;
    // 99 Emotes direkt anlegen, das 100. per API, das 101. scheitert.
    for i in 0..99 {
        sqlx::query(
            "INSERT INTO custom_emojis (id, community_id, name, content_type, size_bytes)
             VALUES ($1, $2, $3, 'image/png', 1)",
        )
        .bind(Uuid::now_v7())
        .bind(Uuid::parse_str(&t.rudel.id).unwrap())
        .bind(format!("e{i}"))
        .execute(&e.app.db)
        .await
        .unwrap();
    }
    e.uploaded(&t.alex, &t.rudel.id, "hundert").await;
    let (status, body) = e.upload(&t.alex, &t.rudel.id, "zuviel", &png(8, 8)).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_emojis"));
    // Andere Communities sind nicht betroffen.
    e.uploaded(&t.carol, &t.andere.id, "zuviel").await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn rename_delete_audit_and_instance_admin(db: PgPool) {
    let e = Emojis::new(db);
    let t = setup(&e.app).await;
    let andere = t.andere.id.as_str();
    let wolf = e.uploaded(&t.carol, andere, "wolf").await;
    let fuchs = e.uploaded(&t.carol, andere, "fuchs").await;
    let wolf_id = s(&wolf["id"]).to_owned();
    let base = format!("/communities/{andere}/emojis/{wolf_id}");

    // Dave ist Mitglied ohne Recht → 403; Bob fremd → 404.
    let (status, _) = e.app.call("PATCH", &base, Some(&t.dave), Some(json!({ "name": "x_y" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = e.app.call("DELETE", &base, Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Umbenennen: ungültig, belegt, ok.
    let (status, body) = e.app.call("PATCH", &base, Some(&t.carol), Some(json!({ "name": "a b" }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_name"));
    let (status, body) = e.app.call("PATCH", &base, Some(&t.carol), Some(json!({ "name": "Fuchs" }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "emoji_name_taken"));
    let (status, renamed) =
        e.app.call("PATCH", &base, Some(&t.carol), Some(json!({ "name": "heuler" }))).await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!((renamed["name"].clone(), renamed["id"].clone()), (json!("heuler"), json!(wolf_id)));

    // Fremde IDs unter falscher Community → 404.
    let (status, _) = e
        .app
        .call("DELETE", &format!("/communities/{}/emojis/{wolf_id}", t.rudel.id), Some(&t.alex), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Instanz-Admin Alex ist kein Mitglied von „Andere“, darf aber verwalten.
    let (status, _) = e.app.call("GET", &format!("/communities/{andere}/emojis"), Some(&t.alex), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = e.app.call("DELETE", &base, Some(&t.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(e.stored_keys().await, vec![s(&fuchs["id"]).to_owned()]);
    let (status, _, _) = e.get_raw(Some(&t.carol), &format!("/api/emojis/{wolf_id}.png")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Audit-Log: create ×2, update, delete.
    let (_, log) = e.app.call("GET", &format!("/communities/{andere}/audit-log"), Some(&t.carol), None).await;
    let actions: Vec<&str> =
        log.as_array().unwrap().iter().map(|a| s(&a["action"])).filter(|a| a.starts_with("emoji.")).collect();
    assert_eq!(actions, vec!["emoji.delete", "emoji.update", "emoji.create", "emoji.create"]);

    // Community löschen entfernt die Emotes (Dateien räumt der Abgleich ab).
    let (status, _) = e.app.call("DELETE", &format!("/communities/{andere}"), Some(&t.carol), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM custom_emojis").fetch_one(&e.app.db).await.unwrap();
    assert_eq!(left, 0);
    let storage = LocalStorage::new(e.dir.path());
    lykos_server::attachments::cleanup(&e.app.db, &storage, std::time::Duration::ZERO).await.unwrap();
    assert!(e.stored_keys().await.is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn messages_keep_own_and_degrade_foreign_emotes(db: PgPool) {
    let e = Emojis::new(db);
    let t = setup(&e.app).await;
    let wolf = e.uploaded(&t.carol, &t.andere.id, "wolf").await;
    let id = s(&wolf["id"]).to_owned();
    let channel = t.rudel.text_channel.as_str();

    // Dave ist Mitglied von „Andere“ → darf das Emote in „Rudel“ nutzen; Präfix und Name werden normalisiert.
    let msg = e.send(&t.dave, channel, &format!("hi <:wolf:{id}> <a:WOLF:{id}>")).await;
    assert_eq!(msg["content"], format!("hi <:wolf:{id}> <:wolf:{id}>"));

    // Bob ist kein Mitglied → `:name:`; unbekannte IDs ebenso; Code bleibt unverändert.
    let unknown = Uuid::now_v7();
    let content = format!("<:wolf:{id}> <:geist:{unknown}> `<:wolf:{id}>`");
    let msg = e.send(&t.bob, channel, &content).await;
    assert_eq!(msg["content"], format!(":wolf: :geist: `<:wolf:{id}>`"));

    // Auch in DMs und beim Bearbeiten.
    let bob_id = e.app.user_id(&t.bob).await;
    let (status, dm) =
        e.app.call("POST", "/dms", Some(&t.dave), Some(json!({ "recipient_ids": [bob_id] }))).await;
    assert!(status.is_success(), "{dm}");
    let dm_msg = e.send(&t.dave, s(&dm["id"]), &format!("<:wolf:{id}>")).await;
    assert_eq!(dm_msg["content"], format!("<:wolf:{id}>"));
    let own = e.send(&t.bob, s(&dm["id"]), "noch nichts").await;
    let (status, edited) = e
        .app
        .call(
            "PATCH",
            &format!("/messages/{}", s(&own["id"])),
            Some(&t.bob),
            Some(json!({ "content": format!("jetzt <:wolf:{id}>") })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edited["content"], "jetzt :wolf:");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reactions_with_custom_emotes(db: PgPool) {
    let e = Emojis::new(db);
    let t = setup(&e.app).await;
    let (status, tanz) = e.upload(&t.carol, &t.andere.id, "tanz", &gif(32, 32)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = s(&tanz["id"]).to_owned();
    let msg = e.send(&t.alex, &t.rudel.text_channel, "Reagiert mal").await;
    let path = |emoji: &str| format!("/messages/{}/reactions/{}/@me", s(&msg["id"]), emoji);
    let key = format!("tanz:{id}");

    // Bob (nicht in „Andere“) darf keine neue Reaktion mit dem Emote setzen.
    let (status, _) = e.app.call("PUT", &path(&key), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Unbekanntes Emote / ungültiger Name.
    let (status, body) =
        e.app.call("PUT", &path(&format!("tanz:{}", Uuid::now_v7())), Some(&t.dave), None).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_emoji"));
    let (status, _) = e.app.call("PUT", &path(&format!("t:{id}")), Some(&t.dave), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Dave (Mitglied) reagiert, danach darf Bob mitklicken – auch mit anderem Namen im Pfad.
    let (status, _) = e.app.call("PUT", &path(&key), Some(&t.dave), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = e.app.call("PUT", &path(&format!("alt:{id}")), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let reactions = |token: String| {
        let e = &e;
        let channel = t.rudel.text_channel.clone();
        async move {
            let (_, list) =
                e.app.call("GET", &format!("/channels/{channel}/messages"), Some(&token), None).await;
            list[0]["reactions"].clone()
        }
    };
    let r = reactions(t.bob.clone()).await;
    assert_eq!(r, json!([{ "emoji": key, "count": 2, "me": true, "emoji_id": id, "animated": true }]));

    // Bob entfernt seine Reaktion wieder.
    let (status, _) = e.app.call("DELETE", &path(&key), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(reactions(t.bob.clone()).await[0]["count"], 1);

    // Emote gelöscht: Reaktion bleibt mit letztem Namen sichtbar; neu reagieren geht nicht mehr.
    let (status, _) = e
        .app
        .call("DELETE", &format!("/communities/{}/emojis/{id}", t.andere.id), Some(&t.carol), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let r = reactions(t.dave.clone()).await;
    assert_eq!(r, json!([{ "emoji": key, "count": 1, "me": true, "emoji_id": id, "animated": false }]));
    // Bestehender Reaktion anschließen geht weiterhin (sie existiert ja).
    let (status, _) = e.app.call("PUT", &path(&key), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Unicode-Reaktionen funktionieren wie bisher und tragen `emoji_id: null`.
    let (status, _) = e.app.call("PUT", &path("%F0%9F%90%BA"), Some(&t.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let r = reactions(t.bob.clone()).await;
    assert_eq!(r[1], json!({ "emoji": "🐺", "count": 1, "me": true, "emoji_id": null, "animated": false }));
}
