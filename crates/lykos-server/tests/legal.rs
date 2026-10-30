//! Rechtstexte der Instanz: `GET /api/legal/{imprint,privacy}` und `legal` in `/instance`.

mod common;

use axum::http::StatusCode;
use common::TestApp;
use lykos_server::config::{Config, LegalTexts};
use sqlx::PgPool;

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn legal_texts_are_served_without_login(db: PgPool) {
    // Wie im Betrieb: Dateien auf der Platte, über die Umgebungsvariablen benannt.
    let dir = tempfile::tempdir().unwrap();
    let imprint = dir.path().join("impressum.md");
    std::fs::write(&imprint, "# Impressum\n\nPlatzhalter GmbH\n").unwrap();
    let imprint = imprint.to_str().unwrap().to_owned();
    let mut config = Config::for_tests();
    config.legal =
        LegalTexts::from_lookup(|name| (name == "LYKOS_IMPRINT_FILE").then(|| imprint.clone())).unwrap();
    let app = TestApp::with_config(db, config);

    let (_, instance) = app.call("GET", "/instance", None, None).await;
    assert_eq!(instance["legal"]["imprint"], true);
    assert_eq!(instance["legal"]["privacy"], false);

    let (status, body) = app.call("GET", "/legal/imprint", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["content"], "# Impressum\n\nPlatzhalter GmbH\n");

    let (status, body) = app.call("GET", "/legal/privacy", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn instance_without_legal_texts(db: PgPool) {
    let app = TestApp::new(db);
    let (_, instance) = app.call("GET", "/instance", None, None).await;
    assert_eq!(instance["legal"]["imprint"], false);
    assert_eq!(instance["legal"]["privacy"], false);
    let (status, _) = app.call("GET", "/legal/imprint", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
