//! Integrationstests gegen einen echten S3-kompatiblen Speicher (Garage, MinIO, …).
//!
//! Laufen nur, wenn `LYKOS_TEST_S3_BUCKET` gesetzt ist – sonst werden sie übersprungen. Die übrigen
//! Variablen wie beim Server, nur mit `LYKOS_TEST_S3_` statt `LYKOS_S3_`:
//!
//! ```sh
//! LYKOS_TEST_S3_ENDPOINT=http://127.0.0.1:9000 LYKOS_TEST_S3_BUCKET=lykos-test \
//! LYKOS_TEST_S3_ACCESS_KEY_ID=… LYKOS_TEST_S3_SECRET_ACCESS_KEY=… cargo test --test s3
//! ```
//!
//! Jeder Test arbeitet unter einem eigenen, zufälligen Präfix und räumt hinterher auf.

mod common;

use std::io;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use bytes::Bytes;
use common::TestApp;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use lykos_server::{
    config::{Config, S3Config, StorageBackend},
    s3_storage::S3Storage,
    storage::{ByteStream, Storage},
};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

/// S3-Konfiguration aus `LYKOS_TEST_S3_*` mit eigenem Präfix; `None` = Tests überspringen.
fn s3_config() -> Option<S3Config> {
    std::env::var("LYKOS_TEST_S3_BUCKET").ok().filter(|b| !b.is_empty())?;
    let backend = StorageBackend::from_lookup(|name| match name {
        "LYKOS_STORAGE" => Some("s3".into()),
        "LYKOS_S3_PREFIX" => Some(format!("lykos-test/{}", Uuid::now_v7())),
        other => std::env::var(other.replace("LYKOS_S3_", "LYKOS_TEST_S3_")).ok(),
    })
    .expect("LYKOS_TEST_S3_* unvollständig");
    match backend {
        StorageBackend::S3(config) => Some(config),
        StorageBackend::Local => unreachable!(),
    }
}

fn stream(parts: Vec<Bytes>) -> ByteStream<'static> {
    Box::pin(futures_util::stream::iter(parts.into_iter().map(Ok)))
}

async fn read_all(s: ByteStream<'static>) -> Vec<u8> {
    s.map(|c| c.unwrap().to_vec()).concat().await
}

#[tokio::test]
async fn s3_roundtrip_range_list_delete() {
    let Some(config) = s3_config() else {
        eprintln!("LYKOS_TEST_S3_BUCKET nicht gesetzt – S3-Test übersprungen");
        return;
    };
    lykos_server::install_crypto_provider();
    let storage = S3Storage::new(&config).unwrap();

    // Kleine Datei: einfacher PUT.
    let small = Uuid::now_v7().to_string();
    let written =
        storage.put(&small, stream(vec![Bytes::from_static(b"hallo "), Bytes::from_static(b"welt")]));
    assert_eq!(written.await.unwrap(), 10);
    assert_eq!(read_all(storage.get(&small, None).await.unwrap().unwrap()).await, b"hallo welt");
    assert_eq!(read_all(storage.get(&small, Some(6..10)).await.unwrap().unwrap()).await, b"welt");

    // Große Datei (> Multipart-Schwelle) in vielen Stücken.
    let big = Uuid::now_v7().to_string();
    let chunk: Vec<u8> = (0..=255u8).cycle().take(1024 * 1024).collect();
    let parts: Vec<Bytes> = (0..20).map(|_| Bytes::from(chunk.clone())).collect();
    assert_eq!(storage.put(&big, stream(parts)).await.unwrap(), 20 * 1024 * 1024);
    let tail =
        read_all(storage.get(&big, Some(20 * 1024 * 1024 - 4..20 * 1024 * 1024)).await.unwrap().unwrap())
            .await;
    assert_eq!(tail, vec![252, 253, 254, 255]);

    let mut keys: Vec<String> = storage.list().await.unwrap().into_iter().map(|o| o.key).collect();
    keys.sort();
    let mut expected = vec![small.clone(), big.clone()];
    expected.sort();
    assert_eq!(keys, expected);

    storage.delete(&small).await.unwrap();
    storage.delete(&small).await.unwrap(); // fehlend ist kein Fehler
    storage.delete(&big).await.unwrap();
    assert!(storage.get(&small, None).await.unwrap().is_none());
    assert!(storage.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn s3_failed_upload_leaves_nothing() {
    let Some(config) = s3_config() else {
        eprintln!("LYKOS_TEST_S3_BUCKET nicht gesetzt – S3-Test übersprungen");
        return;
    };
    lykos_server::install_crypto_provider();
    let storage = S3Storage::new(&config).unwrap();
    let key = Uuid::now_v7().to_string();

    // Abbruch nach 12 MiB → Multipart-Upload wird verworfen.
    let mut parts: Vec<io::Result<Bytes>> =
        (0..12).map(|_| Ok(Bytes::from(vec![7u8; 1024 * 1024]))).collect();
    parts.push(Err(io::Error::other("abgebrochen")));
    let failing: ByteStream<'static> = Box::pin(futures_util::stream::iter(parts));
    assert!(storage.put(&key, failing).await.is_err());
    assert!(storage.get(&key, None).await.unwrap().is_none());
    assert!(storage.list().await.unwrap().is_empty());
}

/// Ende-zu-Ende über die API: Upload, Download mit `Range` (206) und ungültigem Bereich (416).
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn s3_attachments_via_api(db: PgPool) {
    let Some(s3) = s3_config() else {
        eprintln!("LYKOS_TEST_S3_BUCKET nicht gesetzt – S3-Test übersprungen");
        return;
    };
    let mut config = Config::for_tests();
    config.storage = StorageBackend::S3(s3.clone());
    let app = TestApp::with_config(db, config);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;

    let boundary = "lykos-s3-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"zahlen.txt\"\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"0123456789");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{}/attachments", community.text_channel))
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
        .header(header::AUTHORIZATION, format!("Bearer {alex}"))
        .body(Body::from(body))
        .unwrap();
    let (status, attachment) = app.send(req).await;
    assert_eq!(status, StatusCode::CREATED, "{attachment}");
    let url = attachment["url"].as_str().unwrap().to_owned();

    let get = |range: Option<&'static str>| {
        let mut req = Request::builder().uri(&url).header(header::AUTHORIZATION, format!("Bearer {alex}"));
        if let Some(r) = range {
            req = req.header(header::RANGE, r);
        }
        let router = app.router.clone();
        async move {
            let res = router.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
            let status = res.status();
            let range = res.headers().get(header::CONTENT_RANGE).map(|v| v.to_str().unwrap().to_owned());
            (status, range, res.into_body().collect().await.unwrap().to_bytes().to_vec())
        }
    };
    assert_eq!(get(None).await, (StatusCode::OK, None, b"0123456789".to_vec()));
    assert_eq!(
        get(Some("bytes=2-4")).await,
        (StatusCode::PARTIAL_CONTENT, Some("bytes 2-4/10".into()), b"234".to_vec())
    );
    assert_eq!(get(Some("bytes=-3")).await.2, b"789");
    assert_eq!(get(Some("bytes=50-")).await.0, StatusCode::RANGE_NOT_SATISFIABLE);

    // Entwurf verwerfen löscht das Objekt im Bucket.
    let id = attachment["id"].as_str().unwrap();
    let (status, _) = app.call("DELETE", &format!("/attachments/{id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(S3Storage::new(&s3).unwrap().list().await.unwrap().is_empty());
}
