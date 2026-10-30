//! Integrationstests für Speicher-Kontingente (Paket E): pro Nutzer und pro Community.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{TestApp, TestCommunity, s};
use lykos_server::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;

const BOUNDARY: &str = "lykos-quota-boundary";
const KB: u64 = 1024;
const MB: u64 = 1024 * 1024;

struct Quota {
    app: TestApp,
    _dir: TempDir,
}

impl Quota {
    fn new(db: PgPool, user_quota_bytes: Option<u64>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_tests();
        config.storage_dir = dir.path().to_path_buf();
        config.user_quota_bytes = user_quota_bytes;
        Self { app: TestApp::with_config(db, config), _dir: dir }
    }

    async fn upload(&self, token: &str, channel: &str, size: usize) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"daten.bin\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend(std::iter::repeat_n(b'x', size));
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel}/attachments"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        self.app.send(req).await
    }

    async fn usage(&self, token: &str) -> Value {
        let (status, body) = self.app.call("GET", "/auth/me/usage", Some(token), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn setup(&self) -> (String, TestCommunity) {
        let alex = self.app.user("alex", None).await;
        let community = self.app.community(&alex, "Rudel").await;
        (alex, community)
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn user_quota_limits_uploads(db: PgPool) {
    let q = Quota::new(db, Some(100 * KB));
    let (alex, rudel) = q.setup().await;
    let channel = &rudel.text_channel;

    assert_eq!(q.usage(&alex).await, json!({ "used_bytes": 0, "quota_bytes": 100 * KB }));

    let (status, first) = q.upload(&alex, channel, 60 * KB as usize).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(q.usage(&alex).await["used_bytes"], 60 * KB);

    // Passt nicht mehr: verständlicher Fehlercode, nichts wird gespeichert.
    let (status, body) = q.upload(&alex, channel, 50 * KB as usize).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "quota_exceeded");
    assert_eq!(q.usage(&alex).await["used_bytes"], 60 * KB);

    // Was noch passt, geht.
    let (status, _) = q.upload(&alex, channel, 40 * KB as usize).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = q.upload(&alex, channel, 1).await;
    assert_eq!((status, body["error"].clone()), (StatusCode::PAYLOAD_TOO_LARGE, json!("quota_exceeded")));

    // Verworfener Entwurf gibt Platz frei.
    let (status, _) =
        q.app.call("DELETE", &format!("/attachments/{}", s(&first["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(q.usage(&alex).await["used_bytes"], 40 * KB);
    let (status, _) = q.upload(&alex, channel, 50 * KB as usize).await;
    assert_eq!(status, StatusCode::CREATED);

    // Kontingente sind pro Nutzer: Bob hat noch alles frei.
    let bob = q.app.invited_user(&alex, &rudel, "bob").await;
    assert_eq!(q.usage(&bob).await["used_bytes"], 0);
    let (status, _) = q.upload(&bob, channel, 90 * KB as usize).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn unlimited_user_quota(db: PgPool) {
    let q = Quota::new(db, None);
    let (alex, rudel) = q.setup().await;
    let (status, _) = q.upload(&alex, &rudel.text_channel, 10 * KB as usize).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(q.usage(&alex).await, json!({ "used_bytes": 10 * KB, "quota_bytes": null }));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn community_quota(db: PgPool) {
    let q = Quota::new(db, None);
    let (alex, rudel) = q.setup().await;
    let bob = q.app.invited_user(&alex, &rudel, "bob").await;
    let channel = &rudel.text_channel;
    let path = format!("/communities/{}", rudel.id);

    // Standard: kein Community-Kontingent.
    let (_, detail) = q.app.call("GET", &path, Some(&alex), None).await;
    assert_eq!(detail["community"]["upload_quota_mb"], Value::Null);

    // Nur mit MANAGE_COMMUNITY einstellbar, nur sinnvolle Werte.
    let (status, _) = q.app.call("PATCH", &path, Some(&bob), Some(json!({ "upload_quota_mb": 1 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for bad in [json!(0), json!(-5), json!(20_000_000)] {
        let (status, _) =
            q.app.call("PATCH", &path, Some(&alex), Some(json!({ "upload_quota_mb": bad }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, community) =
        q.app.call("PATCH", &path, Some(&alex), Some(json!({ "upload_quota_mb": 1 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(community["upload_quota_mb"], 1);

    // Uploads aller Mitglieder zählen gemeinsam.
    let (status, _) = q.upload(&alex, channel, (600 * KB) as usize).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = q.upload(&bob, channel, (500 * KB) as usize).await;
    assert_eq!((status, body["error"].clone()), (StatusCode::PAYLOAD_TOO_LARGE, json!("quota_exceeded")));
    let (status, _) = q.upload(&bob, channel, (400 * KB) as usize).await;
    assert_eq!(status, StatusCode::CREATED);

    // Belegung der Community: nur mit MANAGE_COMMUNITY.
    let usage_path = format!("/communities/{}/usage", rudel.id);
    let (status, usage) = q.app.call("GET", &usage_path, Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage, json!({ "used_bytes": 1000 * KB, "quota_bytes": MB }));
    let (status, _) = q.app.call("GET", &usage_path, Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Kontingent aufheben → wieder unbegrenzt.
    let (_, community) =
        q.app.call("PATCH", &path, Some(&alex), Some(json!({ "upload_quota_mb": null }))).await;
    assert_eq!(community["upload_quota_mb"], Value::Null);
    let (status, _) = q.upload(&bob, channel, (500 * KB) as usize).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn usage_requires_login(db: PgPool) {
    let q = Quota::new(db, Some(MB));
    let (status, _) = q.app.call("GET", "/auth/me/usage", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Die zweite Prüfung beim Eintragen (unter Sperre) greift auch, wenn die Vorab-Prüfung
/// zwei parallele Uploads durchgelassen hat.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn insert_rechecks_quota(db: PgPool) {
    let mut config = Config::for_tests();
    config.user_quota_bytes = Some(100 * KB);
    let state = lykos_server::state(db.clone(), config);
    let app = TestApp { router: lykos_server::app(state.clone()).unwrap(), db };
    let alex = app.user("alex", None).await;
    let rudel = app.community(&alex, "Rudel").await;
    let alex_id: uuid::Uuid = app.user_id(&alex).await.parse().unwrap();

    let new = |size: u64| lykos_server::attachments::NewAttachment {
        id: uuid::Uuid::now_v7(),
        community_id: Some(rudel.id.parse().unwrap()),
        channel_id: rudel.text_channel.parse().unwrap(),
        uploader_id: alex_id,
        filename: "a.bin",
        content_type: "application/octet-stream",
        size_bytes: size as i64,
        dimensions: None,
    };
    lykos_server::quota::insert_within_quota(&state, new(80 * KB)).await.unwrap();
    let err = lykos_server::quota::insert_within_quota(&state, new(30 * KB)).await.unwrap_err();
    assert!(matches!(err, lykos_server::error::ApiError::PayloadTooLarge("quota_exceeded")), "{err:?}");
    lykos_server::quota::insert_within_quota(&state, new(20 * KB)).await.unwrap();
}
