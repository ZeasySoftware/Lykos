//! Integrationstests für Dateianhänge (Paket C). Speicher liegt pro Test in einem Temp-Verzeichnis.

mod common;

use std::time::Duration;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use http_body_util::BodyExt;
use lykos_server::{
    attachments,
    config::Config,
    storage::{LocalStorage, Storage},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;

const BOUNDARY: &str = "lykos-test-boundary-7f3a";

/// Minimaler PNG-Kopf (Signatur + IHDR) mit 640×480 – reicht für Typ-Erkennung und Maße.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0");
    v.extend_from_slice(&[0u8; 64]);
    v
}

struct Files {
    app: TestApp,
    dir: TempDir,
}

impl Files {
    fn new(db: PgPool) -> Self {
        Self::with_limit(db, 100 * 1024 * 1024)
    }

    fn with_limit(db: PgPool, max_upload_bytes: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_tests();
        config.storage_dir = dir.path().to_path_buf();
        config.max_upload_bytes = max_upload_bytes;
        Self { app: TestApp::with_config(db, config), dir }
    }

    fn storage(&self) -> LocalStorage {
        LocalStorage::new(self.dir.path())
    }

    async fn stored_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.storage().list().await.unwrap().into_iter().map(|o| o.key).collect();
        keys.sort();
        keys
    }

    async fn upload_field(
        &self,
        token: Option<&str>,
        channel: &str,
        field: &str,
        filename: &str,
        data: &[u8],
    ) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel}/attachments"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"));
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        self.app.send(req.body(Body::from(body)).unwrap()).await
    }

    async fn upload(&self, token: &str, channel: &str, filename: &str, data: &[u8]) -> (StatusCode, Value) {
        self.upload_field(Some(token), channel, "file", filename, data).await
    }

    /// Lädt einen Anhang erfolgreich hoch und gibt das Attachment-Objekt zurück.
    async fn uploaded(&self, token: &str, channel: &str, filename: &str, data: &[u8]) -> Value {
        let (status, body) = self.upload(token, channel, filename, data).await;
        assert_eq!(status, StatusCode::CREATED, "Upload {filename}: {body}");
        body
    }

    /// GET mit Rohdaten und Headern.
    async fn download(
        &self,
        token: &str,
        url: &str,
        range: Option<&str>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder()
            .method("GET")
            .uri(url)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        if let Some(range) = range {
            req = req.header(header::RANGE, range);
        }
        let res = self.app.router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        (status, headers, bytes)
    }

    async fn send_message(&self, token: &str, channel: &str, body: Value) -> (StatusCode, Value) {
        self.app.call("POST", &format!("/channels/{channel}/messages"), Some(token), Some(body)).await
    }

    async fn setup(&self) -> (String, TestCommunity) {
        let owner = self.app.user("alex", None).await;
        let community = self.app.community(&owner, "Rudel").await;
        (owner, community)
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn upload_requires_rights_and_detects_type(db: PgPool) {
    let f = Files::new(db);
    let (alex, rudel) = f.setup().await;
    let bob = f.app.invited_user(&alex, &rudel, "bob").await;

    // Carol ist nur Mitglied einer anderen Community.
    let carol = f.app.user("carol", Some(&invite_to_own_community(&f, &alex).await)).await;

    // Bild: Typ aus Magic Bytes, Maße ausgelesen, Dateiname gesäubert.
    let (status, a) = f.upload(&bob, &rudel.text_channel, "../../geheim/urlaub.png", &png(640, 480)).await;
    assert_eq!(status, StatusCode::CREATED, "{a}");
    assert_eq!(a["content_type"], "image/png");
    assert_eq!(a["width"], 640);
    assert_eq!(a["height"], 480);
    assert_eq!(a["filename"], "urlaub.png");
    assert!(a["message_id"].is_null());
    assert_eq!(a["size_bytes"], png(640, 480).len());
    assert_eq!(s(&a["url"]), format!("/api/attachments/{}/urlaub.png", s(&a["id"])));

    // Endung lügt: Client sagt .png/image/png, Inhalt ist HTML bzw. unbekannt.
    let (_, html) =
        f.upload(&bob, &rudel.text_channel, "bild.png", b"<html><script>alert(1)</script></html>").await;
    assert_eq!(html["content_type"], "text/html");
    assert!(html["width"].is_null());
    let (_, unknown) = f.upload(&bob, &rudel.text_channel, "notiz.png", b"nur ein bisschen text").await;
    assert_eq!(unknown["content_type"], "application/octet-stream");

    // Ohne Anmeldung, Voice-Kanal, falsches Feld.
    assert_eq!(
        f.upload_field(None, &rudel.text_channel, "file", "x.png", b"x").await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = f.upload(&bob, &rudel.voice_channel, "x.png", b"x").await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "not_a_text_channel"));
    let (status, _) = f.upload_field(Some(&bob), &rudel.text_channel, "datei", "x.png", b"x").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nicht-Mitglied: 404 (Existenz nicht verraten).
    let (status, _) = f.upload(&carol, &rudel.text_channel, "x.png", b"x").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Standardrolle ohne ATTACH_FILES: 403.
    let perms = 1 | 2; // VIEW_CHANNEL | SEND_MESSAGES
    let (status, _) = f
        .app
        .call(
            "PATCH",
            &format!("/roles/{}", rudel.default_role),
            Some(&alex),
            Some(json!({ "permissions": perms })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = f.upload(&bob, &rudel.text_channel, "x.png", b"x").await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "forbidden"));
    // Besitzer darf weiterhin.
    assert_eq!(f.upload(&alex, &rudel.text_channel, "x.png", b"x").await.0, StatusCode::CREATED);

    assert_eq!(f.stored_keys().await.len(), 4);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn upload_size_limit(db: PgPool) {
    let f = Files::with_limit(db, 1024);
    let (alex, rudel) = f.setup().await;

    assert_eq!(f.upload(&alex, &rudel.text_channel, "ok.bin", &[7u8; 1024]).await.0, StatusCode::CREATED);

    let (status, body) = f.upload(&alex, &rudel.text_channel, "gross.bin", &[7u8; 1025]).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "file_too_large");

    // Groß genug, dass schon Content-Length abgewiesen wird.
    let (status, body) = f.upload(&alex, &rudel.text_channel, "riesig.bin", &vec![7u8; 200 * 1024]).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::PAYLOAD_TOO_LARGE, "file_too_large"));

    // Nur die erlaubte Datei liegt im Speicher, nichts Halbfertiges.
    assert_eq!(f.stored_keys().await.len(), 1);
    let (_, instance) = f.app.call("GET", "/instance", None, None).await;
    assert_eq!(instance["max_upload_bytes"], 1024);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn message_with_attachments(db: PgPool) {
    let f = Files::new(db);
    let (alex, rudel) = f.setup().await;
    let bob = f.app.invited_user(&alex, &rudel, "bob").await;
    let ch = &rudel.text_channel;

    let a1 = f.uploaded(&alex, ch, "eins.png", &png(10, 20)).await;
    let a2 = f.uploaded(&alex, ch, "zwei.pdf", b"%PDF-1.7\n...").await;
    let ids = json!([s(&a1["id"]), s(&a2["id"])]);

    // Ohne Text und ohne Anhang: weiterhin ungültig.
    let (status, body) = f.send_message(&alex, ch, json!({ "content": "  " })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_content"));

    // Fremder Upload lässt sich nicht verwenden.
    let (status, body) =
        f.send_message(&bob, ch, json!({ "content": "", "attachment_ids": [s(&a1["id"])] })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_attachments"));

    // Leerer Text ist mit Anhängen erlaubt.
    let (status, msg) = f.send_message(&alex, ch, json!({ "content": "", "attachment_ids": ids })).await;
    assert_eq!(status, StatusCode::CREATED, "{msg}");
    assert_eq!(msg["content"], "");
    let list = msg["attachments"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["filename"], "eins.png");
    assert_eq!(list[0]["width"], 10);
    assert_eq!(list[0]["message_id"], msg["id"]);
    assert_eq!(list[1]["content_type"], "application/pdf");
    assert!(s(&list[1]["url"]).ends_with("/zwei.pdf"));

    // Schon verknüpft: nicht ein zweites Mal verwendbar – und es entsteht keine Nachricht.
    let (status, body) =
        f.send_message(&alex, ch, json!({ "content": "nochmal", "attachment_ids": ids })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_attachments"));

    // Mehr als 10.
    let many: Vec<String> = (0..11).map(|_| uuid::Uuid::now_v7().to_string()).collect();
    let (status, body) = f.send_message(&alex, ch, json!({ "content": "x", "attachment_ids": many })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "too_many_attachments"));

    // Liste lädt Anhänge mit (und Nachrichten ohne Anhang haben eine leere Liste).
    f.send_message(&bob, ch, json!({ "content": "schön!" })).await;
    let (_, page) = f.app.call("GET", &format!("/channels/{ch}/messages"), Some(&bob), None).await;
    let page = page.as_array().unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0]["attachments"], json!([]));
    assert_eq!(page[1]["attachments"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn download_rights_and_headers(db: PgPool) {
    let f = Files::new(db);
    let (alex, rudel) = f.setup().await;
    let bob = f.app.invited_user(&alex, &rudel, "bob").await;
    // Dave ist Mitglied einer anderen Community, nicht von „Rudel“.
    let dave = f.app.user("dave", Some(&invite_to_own_community(&f, &alex).await)).await;
    let ch = &rudel.text_channel;

    let image = png(3, 4);
    let img = f.uploaded(&alex, ch, "bild.png", &image).await;
    let doc = f.uploaded(&alex, ch, "seite.html", b"<html><body>hi</body></html>").await;
    let (status, _) = f
        .send_message(&alex, ch, json!({ "content": "", "attachment_ids": [s(&img["id"]), s(&doc["id"])] }))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Bild inline, mit korrektem Typ.
    let (status, h, bytes) = f.download(&bob, s(&img["url"]), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, image);
    assert_eq!(h[header::CONTENT_TYPE], "image/png");
    assert!(h[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("inline;"));
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(h[header::CACHE_CONTROL].to_str().unwrap().starts_with("private"));
    // Kein `sandbox` für Bilder – nur die allgemeine CSP der Instanz (`security_headers`).
    assert!(h[header::CONTENT_SECURITY_POLICY].to_str().unwrap().starts_with("default-src 'self'"));

    // HTML nie inline und nie als HTML.
    let (status, h, _) = f.download(&bob, s(&doc["url"]), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "application/octet-stream");
    assert!(h[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment;"));
    assert_eq!(h[header::CONTENT_SECURITY_POLICY], "sandbox");

    // Range.
    let (status, h, bytes) = f.download(&bob, s(&img["url"]), Some("bytes=0-7")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, &image[..8]);
    assert_eq!(h[header::CONTENT_RANGE], format!("bytes 0-7/{}", image.len()));
    let (status, _, _) = f.download(&bob, s(&img["url"]), Some("bytes=9999-")).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);

    // Fremder: 404. Unbekannte ID: 404.
    assert_eq!(f.download(&dave, s(&img["url"]), None).await.0, StatusCode::NOT_FOUND);
    let unknown = format!("/api/attachments/{}/x.png", uuid::Uuid::now_v7());
    assert_eq!(f.download(&bob, &unknown, None).await.0, StatusCode::NOT_FOUND);

    // Unverknüpfter Upload: nur der Uploader sieht ihn.
    let draft = f.uploaded(&bob, ch, "entwurf.png", &png(1, 1)).await;
    assert_eq!(f.download(&bob, s(&draft["url"]), None).await.0, StatusCode::OK);
    assert_eq!(f.download(&alex, s(&draft["url"]), None).await.0, StatusCode::NOT_FOUND);

    // Kanal privat machen: Bob verliert den Zugriff auf die Dateien.
    let (status, _) = f
        .app
        .call(
            "PUT",
            &format!("/channels/{ch}/overrides/{}", rudel.default_role),
            Some(&alex),
            Some(json!({ "allow": 0, "deny": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(f.download(&bob, s(&img["url"]), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.download(&alex, s(&img["url"]), None).await.0, StatusCode::OK);
}

/// Alex legt eine zweite Community an und gibt einen Einladungscode dafür zurück.
async fn invite_to_own_community(f: &Files, owner: &str) -> String {
    let other = f.app.community(owner, "Andere").await;
    let (_, invite) =
        f.app.call("POST", &format!("/communities/{}/invites", other.id), Some(owner), Some(json!({}))).await;
    s(&invite["code"]).to_owned()
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn deleting_cleans_up_files(db: PgPool) {
    let f = Files::new(db);
    let (alex, rudel) = f.setup().await;
    let ch = &rudel.text_channel;
    let storage = f.storage();

    // Nachricht löschen entfernt Zeilen und Dateien sofort.
    let a = f.uploaded(&alex, ch, "a.png", &png(1, 1)).await;
    let (_, msg) =
        f.send_message(&alex, ch, json!({ "content": "x", "attachment_ids": [s(&a["id"])] })).await;
    assert_eq!(f.stored_keys().await, vec![s(&a["id"]).to_owned()]);
    let (status, _) = f.app.call("DELETE", &format!("/messages/{}", s(&msg["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(f.stored_keys().await.is_empty());
    assert_eq!(f.download(&alex, s(&a["url"]), None).await.0, StatusCode::NOT_FOUND);

    // Verworfener Entwurf (DELETE durch den Uploader).
    let draft = f.uploaded(&alex, ch, "d.png", &png(1, 1)).await;
    let path = format!("/attachments/{}", s(&draft["id"]));
    assert_eq!(f.app.call("DELETE", &path, Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(f.app.call("DELETE", &path, Some(&alex), None).await.0, StatusCode::NOT_FOUND);
    assert!(f.stored_keys().await.is_empty());

    // Unverknüpfte Uploads älter als 24 h räumt der Job weg, frische nicht.
    let old = f.uploaded(&alex, ch, "alt.png", &png(1, 1)).await;
    let fresh = f.uploaded(&alex, ch, "neu.png", &png(1, 1)).await;
    sqlx::query("UPDATE attachments SET created_at = now() - interval '25 hours' WHERE id = $1::uuid")
        .bind(s(&old["id"]))
        .execute(&f.app.db)
        .await
        .unwrap();
    attachments::cleanup(&f.app.db, &storage, attachments::ORPHAN_GRACE).await.unwrap();
    assert_eq!(f.stored_keys().await, vec![s(&fresh["id"]).to_owned()]);

    // Aufbewahrungsfrist der Community gilt auch für Anhänge.
    let kept = f.uploaded(&alex, ch, "k.png", &png(1, 1)).await;
    let (_, msg) =
        f.send_message(&alex, ch, json!({ "content": "alt", "attachment_ids": [s(&kept["id"])] })).await;
    sqlx::query("UPDATE messages SET created_at = now() - interval '10 days' WHERE id = $1::uuid")
        .bind(s(&msg["id"]))
        .execute(&f.app.db)
        .await
        .unwrap();
    attachments::cleanup(&f.app.db, &storage, attachments::ORPHAN_GRACE).await.unwrap();
    assert_eq!(f.stored_keys().await.len(), 2, "ohne Frist bleibt alles");
    f.app
        .call(
            "PATCH",
            &format!("/communities/{}", rudel.id),
            Some(&alex),
            Some(json!({ "message_retention_days": 7 })),
        )
        .await;
    attachments::cleanup(&f.app.db, &storage, attachments::ORPHAN_GRACE).await.unwrap();
    assert_eq!(f.stored_keys().await, vec![s(&fresh["id"]).to_owned()]);

    // Community löschen: DB-Kaskade, Dateien gleicht der Job ab (verwaist = ohne DB-Zeile).
    let (status, _) = f.app.call("DELETE", &format!("/communities/{}", rudel.id), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments").fetch_one(&f.app.db).await.unwrap();
    assert_eq!(count, 0);
    // Frische Dateien sind vor dem Abgleich geschützt (laufende Uploads) …
    attachments::cleanup(&f.app.db, &storage, attachments::ORPHAN_GRACE).await.unwrap();
    assert_eq!(f.stored_keys().await.len(), 1);
    // … nach Ablauf der Schonfrist werden sie entfernt.
    attachments::cleanup(&f.app.db, &storage, Duration::ZERO).await.unwrap();
    assert!(f.stored_keys().await.is_empty());
}
