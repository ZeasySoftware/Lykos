//! Integrationstests für Paket J: Community-Icon/-Banner/-Beschreibung/-Akzentfarbe, Systemkanal mit
//! Beitritts-Meldungen, Profilbanner/-farben/Pronomen, Sichtbarkeit (inkl. öffentlicher Einladungsseite),
//! Datenexport, Konto löschen und Aufräum-Job.

mod common;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use common::{PASSWORD, TestApp, TestCommunity, s};
use http_body_util::BodyExt;
use lykos_server::{
    config::Config,
    storage::{LocalStorage, Storage},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;

const BOUNDARY: &str = "lykos-test-boundary-customization";
const MANAGE_COMMUNITY: i64 = 1 << 11;

/// Minimaler PNG-Kopf (Signatur + IHDR) – reicht für Typ-Erkennung und Maße.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0");
    v.extend_from_slice(&[0u8; 64]);
    v
}

/// Minimales GIF (Kopf + logischer Bildschirm) – animierte Icons werden unverändert angenommen.
fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend_from_slice(&width.to_le_bytes());
    v.extend_from_slice(&height.to_le_bytes());
    v.extend_from_slice(&[0, 0, 0]);
    v.extend_from_slice(&[0u8; 32]);
    v
}

struct Fixture {
    app: TestApp,
    dir: TempDir,
}

impl Fixture {
    fn new(db: PgPool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_tests();
        config.storage_dir = dir.path().to_path_buf();
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

    async fn upload(&self, token: &str, path: &str, data: &[u8]) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"bild.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api{path}"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        self.app.send(req).await
    }

    /// GET auf eine absolute API-URL (`/api/...`), optional mit Token.
    async fn get_raw(&self, token: Option<&str>, url: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method("GET").uri(url);
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let res = self.app.router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn invite(&self, token: &str, community: &TestCommunity) -> String {
        let (status, invite) = self
            .app
            .call("POST", &format!("/communities/{}/invites", community.id), Some(token), Some(json!({})))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{invite}");
        s(&invite["code"]).to_owned()
    }
}

/// Alex (Besitzer von „Rudel“), Bob (Mitglied von „Rudel“) und Carol (nur in Alex' zweiter Community).
async fn setup(app: &TestApp) -> (String, String, String, TestCommunity) {
    let alex = app.user("alex", None).await;
    let rudel = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &rudel, "bob").await;
    let andere = app.community(&alex, "Andere").await;
    let carol = app.invited_user(&alex, &andere, "carol").await;
    (alex, bob, carol, rudel)
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn community_icon_and_banner(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, bob, carol, rudel) = setup(&f.app).await;
    let icon_path = format!("/communities/{}/icon", rudel.id);
    let banner_path = format!("/communities/{}/banner", rudel.id);

    // Ohne MANAGE_COMMUNITY → 403, Fremde → 404.
    assert_eq!(f.upload(&bob, &icon_path, &png(256, 256)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.upload(&carol, &icon_path, &png(256, 256)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.app.call("DELETE", &icon_path, Some(&bob), None).await.0, StatusCode::FORBIDDEN);

    // Falscher Typ, zu große Maße, zu groß.
    let (status, body) = f.upload(&alex, &icon_path, b"<svg onload=alert(1)></svg>").await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_image"));
    assert_eq!(f.upload(&alex, &icon_path, &png(2000, 2000)).await.0, StatusCode::BAD_REQUEST);
    let mut huge = png(256, 256);
    huge.resize(3 * 1024 * 1024, 0);
    assert_eq!(f.upload(&alex, &icon_path, &huge).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(f.upload(&alex, &banner_path, &png(4000, 100)).await.0, StatusCode::BAD_REQUEST);
    assert!(f.stored_keys().await.is_empty());

    // Gültig: Icon (auch GIF) und Banner.
    let (status, community) = f.upload(&alex, &icon_path, &gif(128, 128)).await;
    assert_eq!(status, StatusCode::OK, "{community}");
    let gif_url = s(&community["icon_url"]).to_owned();
    let (status, community) = f.upload(&alex, &icon_path, &png(256, 256)).await;
    assert_eq!(status, StatusCode::OK, "{community}");
    let icon_url = s(&community["icon_url"]).to_owned();
    assert!(icon_url.starts_with(&format!("/api/communities/{}/icon/", rudel.id)), "{icon_url}");
    assert_ne!(gif_url, icon_url);
    let (status, community) = f.upload(&alex, &banner_path, &png(960, 540)).await;
    assert_eq!(status, StatusCode::OK, "{community}");
    let banner_url = s(&community["banner_url"]).to_owned();
    assert_eq!(community["icon_url"], icon_url.as_str());
    // Ersetzen löscht das alte Bild sofort.
    assert_eq!(f.stored_keys().await.len(), 2);
    assert_eq!(f.get_raw(Some(&alex), &gif_url).await.0, StatusCode::NOT_FOUND);

    // Auch die Community-Liste der Mitglieder liefert die URLs.
    let (_, list) = f.app.call("GET", "/communities", Some(&bob), None).await;
    let listed = list.as_array().unwrap().iter().find(|c| c["id"] == rudel.id.as_str()).unwrap();
    assert_eq!(listed["icon_url"], icon_url.as_str());
    assert_eq!(listed["banner_url"], banner_url.as_str());

    // Abruf: Mitglieder ja, Fremde 404, ohne Login 401.
    let (status, headers, bytes) = f.get_raw(Some(&bob), &icon_url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(headers[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
    assert_eq!(bytes, png(256, 256));
    assert_eq!(f.get_raw(Some(&bob), &banner_url).await.0, StatusCode::OK);
    assert_eq!(f.get_raw(Some(&carol), &icon_url).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.get_raw(Some(&carol), &banner_url).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.get_raw(None, &icon_url).await.0, StatusCode::UNAUTHORIZED);

    // Audit-Log.
    let (_, log) =
        f.app.call("GET", &format!("/communities/{}/audit-log", rudel.id), Some(&alex), None).await;
    let actions: Vec<&str> = log.as_array().unwrap().iter().map(|e| s(&e["action"])).collect();
    assert!(actions.contains(&"community.icon_set"), "{actions:?}");
    assert!(actions.contains(&"community.banner_set"), "{actions:?}");

    // Entfernen.
    let (status, community) = f.app.call("DELETE", &icon_path, Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(community["icon_url"].is_null());
    assert_eq!(f.stored_keys().await.len(), 1);
    assert_eq!(f.get_raw(Some(&bob), &icon_url).await.0, StatusCode::NOT_FOUND);
    let (_, log) =
        f.app.call("GET", &format!("/communities/{}/audit-log", rudel.id), Some(&alex), None).await;
    assert_eq!(log[0]["action"], "community.icon_remove");

    // MANAGE_COMMUNITY über eine Rolle genügt.
    let bob_id = f.app.user_id(&bob).await;
    let (_, role) = f
        .app
        .call(
            "POST",
            &format!("/communities/{}/roles", rudel.id),
            Some(&alex),
            Some(json!({ "name": "Gestalter", "permissions": MANAGE_COMMUNITY })),
        )
        .await;
    f.app
        .call(
            "PUT",
            &format!("/communities/{}/members/{bob_id}/roles/{}", rudel.id, s(&role["id"])),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(f.upload(&bob, &icon_path, &png(64, 64)).await.0, StatusCode::OK);

    // Community löschen entfernt die Bilder sofort.
    assert_eq!(f.stored_keys().await.len(), 2);
    assert_eq!(
        f.app.call("DELETE", &format!("/communities/{}", rudel.id), Some(&alex), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(f.stored_keys().await.is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn invite_page_shows_public_appearance(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, _bob, carol, rudel) = setup(&f.app).await;
    f.upload(&alex, &format!("/communities/{}/icon", rudel.id), &png(256, 256)).await;
    f.upload(&alex, &format!("/communities/{}/banner", rudel.id), &png(960, 540)).await;
    let (status, _) = f
        .app
        .call(
            "PATCH",
            &format!("/communities/{}", rudel.id),
            Some(&alex),
            Some(json!({ "description": "Ein Rudel für alle.", "accent_color": 0x7c5cff })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let code = f.invite(&alex, &rudel).await;

    // Öffentliche Vorschau (ohne Login) mit Beschreibung, Farbe und Einladungs-URLs der Bilder.
    let (status, preview) = f.app.call("GET", &format!("/invites/{code}"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let c = &preview["community"];
    assert_eq!(c["description"], "Ein Rudel für alle.");
    assert_eq!(c["accent_color"], 0x7c5cff);
    let icon = s(&c["icon_url"]).to_owned();
    let banner = s(&c["banner_url"]).to_owned();
    assert!(icon.starts_with(&format!("/api/invites/{code}/icon/")), "{icon}");
    let (status, headers, bytes) = f.get_raw(None, &icon).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(bytes, png(256, 256));
    assert_eq!(f.get_raw(None, &banner).await.0, StatusCode::OK);
    // Auch Nicht-Mitglieder mit Login sehen sie über die Einladung, aber nicht über den Community-Pfad.
    assert_eq!(f.get_raw(Some(&carol), &icon).await.0, StatusCode::OK);
    let member_icon =
        icon.replace(&format!("/api/invites/{code}/"), &format!("/api/communities/{}/", rudel.id));
    assert_eq!(f.get_raw(Some(&carol), &member_icon).await.0, StatusCode::NOT_FOUND);

    // Falscher Code oder falsche Bild-ID → 404; nach Löschen der Einladung ebenfalls.
    let wrong = icon.replace(&code, "xxxxxxxx");
    assert_eq!(f.get_raw(None, &wrong).await.0, StatusCode::NOT_FOUND);
    let (_, community) = f.app.call("GET", &format!("/communities/{}", rudel.id), Some(&alex), None).await;
    let banner_id = s(&community["community"]["banner_url"]).rsplit('/').next().unwrap().to_owned();
    assert_eq!(
        f.get_raw(None, &format!("/api/invites/{code}/icon/{banner_id}")).await.0,
        StatusCode::NOT_FOUND
    );
    f.app.call("DELETE", &format!("/invites/{code}"), Some(&alex), None).await;
    assert_eq!(f.get_raw(None, &icon).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn community_appearance_fields(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, bob, _carol, rudel) = setup(&f.app).await;
    let path = format!("/communities/{}", rudel.id);
    let patch = |token: String, body: Value| {
        let app = &f.app;
        let path = path.clone();
        async move { app.call("PATCH", &path, Some(&token), Some(body)).await }
    };

    // Ohne Recht.
    assert_eq!(patch(bob.clone(), json!({ "description": "x" })).await.0, StatusCode::FORBIDDEN);
    // Ungültige Werte ändern nichts.
    let (status, body) = patch(alex.clone(), json!({ "description": "a".repeat(301) })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_description"));
    let (status, body) = patch(alex.clone(), json!({ "accent_color": 0x100_0000 })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_color"));
    let (status, body) = patch(alex.clone(), json!({ "accent_color": -1 })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_color"));
    let (status, body) = patch(alex.clone(), json!({ "system_channel_id": rudel.voice_channel })).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_channel"));
    let andere = f.app.community(&alex, "Dritte").await;
    let (status, _) = patch(alex.clone(), json!({ "system_channel_id": andere.text_channel })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Setzen; fehlende Felder bleiben, null entfernt.
    let (status, c) = patch(
        alex.clone(),
        json!({ "description": "  Willkommen!  ", "accent_color": 0x3fb950, "system_channel_id": rudel.text_channel }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["description"], "Willkommen!");
    assert_eq!(c["accent_color"], 0x3fb950);
    assert_eq!(c["system_channel_id"], rudel.text_channel.as_str());
    let (_, c) = patch(alex.clone(), json!({ "name": "Rudel 2" })).await;
    assert_eq!(c["description"], "Willkommen!");
    let (_, c) = patch(alex.clone(), json!({ "description": null, "accent_color": null })).await;
    assert!(c["description"].is_null() && c["accent_color"].is_null());
    assert_eq!(c["system_channel_id"], rudel.text_channel.as_str());

    let (_, log) =
        f.app.call("GET", &format!("/communities/{}/audit-log", rudel.id), Some(&alex), None).await;
    assert_eq!(log[0]["action"], "community.update");
    assert!(log[0]["data"].get("description").is_some(), "{}", log[0]);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn join_message_in_system_channel(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, _bob, _carol, rudel) = setup(&f.app).await;
    let messages_path = format!("/channels/{}/messages", rudel.text_channel);

    // Ohne Systemkanal: keine Meldung (neue Communities nutzen den ersten Textkanal – hier entfernt).
    f.app
        .call(
            "PATCH",
            &format!("/communities/{}", rudel.id),
            Some(&alex),
            Some(json!({ "system_channel_id": null })),
        )
        .await;
    let (_, msgs) = f.app.call("GET", &messages_path, Some(&alex), None).await;
    let before = msgs.as_array().unwrap().len();
    f.app.invited_user(&alex, &rudel, "dora").await;
    let (_, msgs) = f.app.call("GET", &messages_path, Some(&alex), None).await;
    assert_eq!(msgs.as_array().unwrap().len(), before, "{msgs}");

    f.app
        .call(
            "PATCH",
            &format!("/communities/{}", rudel.id),
            Some(&alex),
            Some(json!({ "system_channel_id": rudel.text_channel })),
        )
        .await;

    // Registrierung mit Einladung und Annehmen einer Einladung erzeugen je eine Systemnachricht.
    let erik = f.app.invited_user(&alex, &rudel, "erik").await;
    let erik_id = f.app.user_id(&erik).await;
    let andere = f.app.community(&alex, "Andere2").await;
    let fritz = f.app.invited_user(&alex, &andere, "fritz").await;
    let code = f.invite(&alex, &rudel).await;
    assert_eq!(
        f.app.call("POST", &format!("/invites/{code}/accept"), Some(&fritz), None).await.0,
        StatusCode::OK
    );

    let (_, msgs) = f.app.call("GET", &messages_path, Some(&erik), None).await;
    let msgs = msgs.as_array().unwrap();
    assert_eq!(msgs.len(), before + 2, "{msgs:?}");
    assert!(msgs.iter().all(|m| m["kind"] == "member_join" && m["content"] == ""));
    assert_eq!(msgs[0]["author"]["username"], "fritz");
    assert_eq!(msgs[1]["author"]["id"], erik_id.as_str());

    // Systemnachrichten sind nicht bearbeitbar, aber löschbar (eigene).
    let id = s(&msgs[1]["id"]).to_owned();
    let (status, _) = f
        .app
        .call("PATCH", &format!("/messages/{id}"), Some(&erik), Some(json!({ "content": "Hack" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        f.app.call("DELETE", &format!("/messages/{id}"), Some(&erik), None).await.0,
        StatusCode::NO_CONTENT
    );

    // Normale Nachrichten haben kind = default.
    let (_, m) = f.app.call("POST", &messages_path, Some(&erik), Some(json!({ "content": "Hallo" }))).await;
    assert_eq!(m["kind"], "default");

    // Wird der Kanal gelöscht, fällt der Systemkanal weg – Beitritte funktionieren weiter.
    f.app.call("DELETE", &format!("/channels/{}", rudel.text_channel), Some(&alex), None).await;
    let (_, detail) = f.app.call("GET", &format!("/communities/{}", rudel.id), Some(&alex), None).await;
    assert!(detail["community"]["system_channel_id"].is_null());
    f.app.invited_user(&alex, &rudel, "gerd").await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn profile_banner_colors_and_pronouns(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, bob, carol, rudel) = setup(&f.app).await;
    let bob_id = f.app.user_id(&bob).await;

    // Ungültige Werte.
    let (status, body) =
        f.app.call("PATCH", "/auth/me", Some(&bob), Some(json!({ "pronouns": "x".repeat(41) }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_pronouns"));
    let (status, _) =
        f.app.call("PATCH", "/auth/me", Some(&bob), Some(json!({ "pronouns": "er\nihm" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) =
        f.app.call("PATCH", "/auth/me", Some(&bob), Some(json!({ "banner_color": 0x100_0000 }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_color"));
    let (status, _) = f.app.call("PATCH", "/auth/me", Some(&bob), Some(json!({ "accent_color": -5 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Setzen.
    let (status, me) = f
        .app
        .call(
            "PATCH",
            "/auth/me",
            Some(&bob),
            Some(json!({ "pronouns": " er/ihm ", "banner_color": 0x223344, "accent_color": 0xff8800 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["pronouns"], "er/ihm");
    assert_eq!(me["banner_color"], 0x223344);
    assert_eq!(me["accent_color"], 0xff8800);

    // Banner hochladen: falscher Typ / zu groß / gültig.
    let (status, _) = f.upload(&bob, "/auth/me/banner", b"%PDF-1.4").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut huge = png(600, 240);
    huge.resize(5 * 1024 * 1024, 0);
    assert_eq!(f.upload(&bob, "/auth/me/banner", &huge).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, me) = f.upload(&bob, "/auth/me/banner", &png(600, 240)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let banner = s(&me["banner_url"]).to_owned();
    assert!(banner.starts_with(&format!("/api/users/{bob_id}/banner/")), "{banner}");
    assert_eq!(me["banner_color"], 0x223344);

    // Sichtbarkeit wie beim Avatar.
    assert_eq!(f.get_raw(Some(&alex), &banner).await.0, StatusCode::OK);
    assert_eq!(f.get_raw(Some(&bob), &banner).await.0, StatusCode::OK);
    assert_eq!(f.get_raw(Some(&carol), &banner).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.get_raw(None, &banner).await.0, StatusCode::UNAUTHORIZED);

    // Profil für andere: neue Felder und gemeinsame Communities.
    let (status, profile) = f.app.call("GET", &format!("/users/{bob_id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["pronouns"], "er/ihm");
    assert_eq!(profile["banner_url"], banner.as_str());
    assert_eq!(profile["accent_color"], 0xff8800);
    assert_eq!(profile["mutual_community_ids"], json!([rudel.id]));

    // Datenexport enthält die neuen Felder.
    let req = Request::get("/api/auth/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {bob}"))
        .body(Body::empty())
        .unwrap();
    let res = f.app.router.clone().oneshot(req).await.unwrap();
    let export: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(export["profile"]["pronouns"], "er/ihm");
    assert_eq!(export["profile"]["banner_url"], banner.as_str());
    assert_eq!(export["profile"]["banner_color"], 0x223344);

    // Ersetzen, Entfernen.
    let (_, me) = f.upload(&bob, "/auth/me/banner", &png(600, 240)).await;
    assert_ne!(s(&me["banner_url"]), banner);
    assert_eq!(f.stored_keys().await.len(), 1);
    let (status, me) = f.app.call("DELETE", "/auth/me/banner", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(me["banner_url"].is_null());
    assert!(f.stored_keys().await.is_empty());

    // Konto löschen nimmt das Banner mit.
    f.upload(&bob, "/auth/me/banner", &png(600, 240)).await;
    assert_eq!(f.stored_keys().await.len(), 1);
    let (status, _) =
        f.app.call("DELETE", "/auth/me", Some(&bob), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(f.stored_keys().await.is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn cleanup_keeps_customization_files_and_removes_orphans(db: PgPool) {
    let f = Fixture::new(db);
    let (alex, bob, _carol, rudel) = setup(&f.app).await;
    f.upload(&alex, &format!("/communities/{}/icon", rudel.id), &png(256, 256)).await;
    f.upload(&alex, &format!("/communities/{}/banner", rudel.id), &png(960, 540)).await;
    f.upload(&bob, "/auth/me/banner", &png(600, 240)).await;
    f.upload(&bob, "/auth/me/avatar", &png(256, 256)).await;
    assert_eq!(f.stored_keys().await.len(), 4);

    // Eine verwaiste Datei (z. B. Bild einer Community, deren Löschen abbrach).
    let orphan = uuid::Uuid::now_v7().to_string();
    f.storage()
        .put(&orphan, Box::pin(futures_util::stream::once(async { Ok(bytes::Bytes::from_static(b"x")) })))
        .await
        .unwrap();
    assert_eq!(f.stored_keys().await.len(), 5);

    lykos_server::attachments::cleanup(&f.app.db, &f.storage(), std::time::Duration::ZERO).await.unwrap();
    let keys = f.stored_keys().await;
    assert_eq!(keys.len(), 4, "{keys:?}");
    assert!(!keys.contains(&orphan));
}
