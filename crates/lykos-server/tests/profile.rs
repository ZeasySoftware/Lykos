//! Integrationstests für Konto & Profil: Avatare, Profil-Texte, Passwort, Sitzungen, Nicknames,
//! Besitzübertragung, Datenexport und Konto löschen.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
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

const BOUNDARY: &str = "lykos-test-boundary-avatar";
const CHANGE_NICKNAME: i64 = 1 << 21;
const MANAGE_NICKNAMES: i64 = 1 << 22;

/// Minimaler PNG-Kopf (Signatur + IHDR) – reicht für Typ-Erkennung und Maße.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0");
    v.extend_from_slice(&[0u8; 64]);
    v
}

struct Profiles {
    app: TestApp,
    dir: TempDir,
}

impl Profiles {
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

    async fn upload_avatar(&self, token: &str, data: &[u8]) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"ich.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/me/avatar")
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        self.app.send(req).await
    }

    async fn get_raw(&self, token: &str, url: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let req = Request::builder()
            .method("GET")
            .uri(url)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = self.app.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
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
async fn avatar_upload_visibility_and_replace(db: PgPool) {
    let p = Profiles::new(db);
    let (alex, bob, carol, rudel) = setup(&p.app).await;
    let bob_id = p.app.user_id(&bob).await;

    // Kein Bild bzw. ungültige Maße → 400.
    let (status, body) = p.upload_avatar(&bob, b"<svg onload=alert(1)></svg>").await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_image"));
    let (status, _) = p.upload_avatar(&bob, b"%PDF-1.4 kein bild").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = p.upload_avatar(&bob, &png(5000, 10)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(p.stored_keys().await.is_empty());

    // Gültiges PNG.
    let (status, me) = p.upload_avatar(&bob, &png(256, 256)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let url = s(&me["avatar_url"]).to_owned();
    assert!(url.starts_with(&format!("/api/users/{bob_id}/avatar/")), "{url}");
    assert_eq!(p.stored_keys().await.len(), 1);

    // Avatar-URL steckt in Mitgliederliste und Nachrichten.
    let (_, members) =
        p.app.call("GET", &format!("/communities/{}/members", rudel.id), Some(&alex), None).await;
    let member = members.as_array().unwrap().iter().find(|m| m["user"]["id"] == bob_id.as_str()).unwrap();
    assert_eq!(member["user"]["avatar_url"], url.as_str());
    let (_, msg) = p
        .app
        .call(
            "POST",
            &format!("/channels/{}/messages", rudel.text_channel),
            Some(&bob),
            Some(json!({ "content": "hi" })),
        )
        .await;
    assert_eq!(msg["author"]["avatar_url"], url.as_str());

    // Abruf: selbst und Alex (gemeinsame Community) ja, Carol nein, ohne Login nein.
    let (status, headers, bytes) = p.get_raw(&alex, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(headers[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
    assert_eq!(bytes, png(256, 256));
    assert_eq!(p.get_raw(&bob, &url).await.0, StatusCode::OK);
    assert_eq!(p.get_raw(&carol, &url).await.0, StatusCode::NOT_FOUND);
    let (status, _) = p.app.send(Request::get(&url).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Profil: gleiche Sichtbarkeitsregel.
    let (status, profile) = p.app.call("GET", &format!("/users/{bob_id}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["username"], "bob");
    assert_eq!(profile["avatar_url"], url.as_str());
    assert!(profile.get("is_instance_admin").is_none());
    assert_eq!(
        p.app.call("GET", &format!("/users/{bob_id}"), Some(&carol), None).await.0,
        StatusCode::NOT_FOUND
    );

    // Ersetzen: neue URL, alte Datei weg, alte URL 404.
    let (_, me2) = p.upload_avatar(&bob, &png(128, 128)).await;
    let url2 = s(&me2["avatar_url"]).to_owned();
    assert_ne!(url, url2);
    assert_eq!(p.stored_keys().await.len(), 1);
    assert_eq!(p.get_raw(&alex, &url).await.0, StatusCode::NOT_FOUND);

    // Der Aufräum-Job hält Avatare für bekannt (auch ohne Schonfrist).
    lykos_server::attachments::cleanup(
        &p.app.db,
        &LocalStorage::new(p.dir.path()),
        std::time::Duration::ZERO,
    )
    .await
    .unwrap();
    assert_eq!(p.stored_keys().await.len(), 1);

    // Entfernen.
    let (status, me3) = p.app.call("DELETE", "/auth/me/avatar", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(me3["avatar_url"].is_null());
    assert!(p.stored_keys().await.is_empty());
    assert_eq!(p.get_raw(&alex, &url2).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn profile_texts(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;

    let (status, me) = app
        .call(
            "PATCH",
            "/auth/me",
            Some(&alex),
            Some(json!({ "bio": "  Hallo, ich bin Alex.  ", "status_text": "Am Basteln" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["bio"], "Hallo, ich bin Alex.");
    assert_eq!(me["status_text"], "Am Basteln");

    // Felder, die fehlen, bleiben; null löscht.
    let (_, me) = app.call("PATCH", "/auth/me", Some(&alex), Some(json!({ "display_name": "Alex" }))).await;
    assert_eq!(me["bio"], "Hallo, ich bin Alex.");
    let (_, me) = app.call("PATCH", "/auth/me", Some(&alex), Some(json!({ "status_text": null }))).await;
    assert!(me["status_text"].is_null());
    assert_eq!(me["display_name"], "Alex");

    // Zu lang → 400, nichts geändert.
    let (status, body) = app
        .call(
            "PATCH",
            "/auth/me",
            Some(&alex),
            Some(json!({ "bio": "x".repeat(191), "display_name": "Neu" })),
        )
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_bio"));
    let (status, body) =
        app.call("PATCH", "/auth/me", Some(&alex), Some(json!({ "status_text": "x".repeat(129) }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_status_text"));
    let (_, me) = app.call("GET", "/auth/me", Some(&alex), None).await;
    assert_eq!(me["display_name"], "Alex");
    assert_eq!(me["bio"].as_str().unwrap().chars().count(), 20);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn change_password_and_sessions(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let login = |pw: &'static str| {
        app.call("POST", "/auth/login", None, Some(json!({ "username": "alex", "password": pw })))
    };
    let (_, other) = login(PASSWORD).await;
    let other = s(&other["token"]).to_owned();
    let (_, third) = login(PASSWORD).await;
    let third = s(&third["token"]).to_owned();

    // Falsches altes Passwort, zu kurzes neues.
    let (status, body) = app
        .call(
            "POST",
            "/auth/password",
            Some(&alex),
            Some(json!({ "current_password": "falsch-falsch-falsch", "new_password": "neues-langes-passwort" })),
        )
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "wrong_password"));
    let (status, body) = app
        .call(
            "POST",
            "/auth/password",
            Some(&alex),
            Some(json!({ "current_password": PASSWORD, "new_password": "kurz" })),
        )
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_password"));

    // Ohne Abmelden der anderen Sitzungen.
    let (status, _) = app
        .call(
            "POST",
            "/auth/password",
            Some(&alex),
            Some(json!({
                "current_password": PASSWORD,
                "new_password": "neues-langes-passwort",
                "logout_other_sessions": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&other), None).await.0, StatusCode::OK);
    assert_eq!(login(PASSWORD).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login("neues-langes-passwort").await.0, StatusCode::OK);

    // Standard: andere Sitzungen werden beendet, die aktuelle bleibt.
    let (status, _) = app
        .call(
            "POST",
            "/auth/password",
            Some(&alex),
            Some(json!({ "current_password": "neues-langes-passwort", "new_password": "noch-ein-passwort" })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&alex), None).await.0, StatusCode::OK);
    assert_eq!(app.call("GET", "/auth/me", Some(&other), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(app.call("GET", "/auth/me", Some(&third), None).await.0, StatusCode::UNAUTHORIZED);

    // Alle anderen Sitzungen beenden.
    let (_, fresh) = login("noch-ein-passwort").await;
    let fresh = s(&fresh["token"]).to_owned();
    let (_, sessions) = app.call("GET", "/auth/sessions", Some(&alex), None).await;
    assert_eq!(sessions.as_array().unwrap().len(), 2);
    assert_eq!(app.call("DELETE", "/auth/sessions", Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&fresh), None).await.0, StatusCode::UNAUTHORIZED);
    let (_, sessions) = app.call("GET", "/auth/sessions", Some(&alex), None).await;
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["current"], true);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn nicknames(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, carol, rudel) = setup(&app).await;
    let alex_id = app.user_id(&alex).await;
    let bob_id = app.user_id(&bob).await;
    let members = |id: &str| format!("/communities/{}/members/{id}", rudel.id);

    // Eigenen Nickname setzen (Standardrolle hat CHANGE_NICKNAME).
    let (status, member) =
        app.call("PATCH", &members("@me"), Some(&bob), Some(json!({ "nickname": "  Bobby " }))).await;
    assert_eq!(status, StatusCode::OK, "{member}");
    assert_eq!(member["nickname"], "Bobby");
    let (status, body) =
        app.call("PATCH", &members("@me"), Some(&bob), Some(json!({ "nickname": "x".repeat(33) }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_name"));

    // Fremde Nicknames nur mit MANAGE_NICKNAMES; Nicht-Mitglied 404.
    let (status, _) =
        app.call("PATCH", &members(&alex_id), Some(&bob), Some(json!({ "nickname": "Chef" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        app.call("PATCH", &members("@me"), Some(&carol), Some(json!({ "nickname": "C" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Besitzer darf fremde ändern → Audit-Log; eigene Änderungen nicht im Audit-Log.
    let (status, member) = app
        .call("PATCH", &members(&bob_id), Some(&alex), Some(json!({ "nickname": "Bob der Baumeister" })))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(member["nickname"], "Bob der Baumeister");
    let (_, log) = app.call("GET", &format!("/communities/{}/audit-log", rudel.id), Some(&alex), None).await;
    let entries: Vec<&Value> =
        log.as_array().unwrap().iter().filter(|e| e["action"] == "member.nickname").collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["target_id"], bob_id.as_str());
    assert_eq!(entries[0]["data"]["nickname"], "Bob der Baumeister");

    // Nickname entfernen.
    let (_, member) =
        app.call("PATCH", &members(&bob_id), Some(&alex), Some(json!({ "nickname": null }))).await;
    assert!(member["nickname"].is_null());

    // Ohne CHANGE_NICKNAME in der Standardrolle: 403.
    let (_, role) = app
        .call(
            "PATCH",
            &format!("/roles/{}", rudel.default_role),
            Some(&alex),
            Some(json!({ "permissions": 3 })),
        )
        .await;
    assert_eq!(role["permissions"].as_i64().unwrap() & CHANGE_NICKNAME, 0);
    let (status, _) = app.call("PATCH", &members("@me"), Some(&bob), Some(json!({ "nickname": "B" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Rolle mit MANAGE_NICKNAMES: fremde ja, Besitzer nein.
    let (_, mods) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", rudel.id),
            Some(&alex),
            Some(json!({ "name": "Mods", "permissions": MANAGE_NICKNAMES })),
        )
        .await;
    assert_eq!(mods["permissions"].as_i64().unwrap(), MANAGE_NICKNAMES);
    let dave = app.invited_user(&alex, &rudel, "dave").await;
    let dave_id = app.user_id(&dave).await;
    let (status, _) = app
        .call(
            "PUT",
            &format!("/communities/{}/members/{bob_id}/roles/{}", rudel.id, s(&mods["id"])),
            Some(&alex),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) =
        app.call("PATCH", &members(&dave_id), Some(&bob), Some(json!({ "nickname": "Davey" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) =
        app.call("PATCH", &members("@me"), Some(&bob), Some(json!({ "nickname": "Mod Bob" }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) =
        app.call("PATCH", &members(&alex_id), Some(&bob), Some(json!({ "nickname": "Chef" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        app.call("PATCH", &members("kein-uuid"), Some(&bob), Some(json!({ "nickname": "x" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn migration_grants_change_nickname(db: PgPool) {
    let app = TestApp::new(db.clone());
    let alex = app.user("alex", None).await;
    let rudel = app.community(&alex, "Rudel").await;
    // Bit entfernen und Migration erneut ausführen (idempotent formuliert).
    sqlx::query("UPDATE roles SET permissions = permissions & ~2097152").execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/0009_profiles.sql")).execute(&db).await.unwrap();
    let perms: i64 = sqlx::query_scalar("SELECT permissions FROM roles WHERE id = $1::uuid")
        .bind(&rudel.default_role)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_ne!(perms & CHANGE_NICKNAME, 0);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn transfer_ownership(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, carol, rudel) = setup(&app).await;
    let bob_id = app.user_id(&bob).await;
    let carol_id = app.user_id(&carol).await;
    let transfer = format!("/communities/{}/transfer", rudel.id);

    assert_eq!(
        app.call("POST", &transfer, Some(&bob), Some(json!({ "user_id": bob_id }))).await.0,
        StatusCode::FORBIDDEN
    );
    // Ziel kein Mitglied.
    assert_eq!(
        app.call("POST", &transfer, Some(&alex), Some(json!({ "user_id": carol_id }))).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, community) =
        app.call("POST", &transfer, Some(&alex), Some(json!({ "user_id": bob_id }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(community["owner_id"], bob_id.as_str());

    // Alex ist jetzt normales Mitglied und darf verlassen; Bob nicht mehr.
    let alex_id = app.user_id(&alex).await;
    let (_, detail) = app.call("GET", &format!("/communities/{}", rudel.id), Some(&bob), None).await;
    assert_eq!(detail["community"]["owner_id"], bob_id.as_str());
    let (_, log) = app.call("GET", &format!("/communities/{}/audit-log", rudel.id), Some(&bob), None).await;
    assert!(log.as_array().unwrap().iter().any(|e| e["action"] == "community.transfer"));
    assert_eq!(
        app.call("DELETE", &format!("/communities/{}/members/{alex_id}", rudel.id), Some(&alex), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn data_export(db: PgPool) {
    let app = TestApp::new(db);
    let (alex, bob, _carol, rudel) = setup(&app).await;
    app.call(
        "POST",
        &format!("/channels/{}/messages", rudel.text_channel),
        Some(&bob),
        Some(json!({ "content": "Hallo Rudel" })),
    )
    .await;
    app.call(
        "POST",
        &format!("/channels/{}/messages", rudel.text_channel),
        Some(&alex),
        Some(json!({ "content": "Nicht von Bob" })),
    )
    .await;
    app.call(
        "PATCH",
        &format!("/communities/{}/members/@me", rudel.id),
        Some(&bob),
        Some(json!({ "nickname": "Bobby" })),
    )
    .await;

    let req = Request::get("/api/auth/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {bob}"))
        .body(Body::empty())
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let disposition = res.headers()[header::CONTENT_DISPOSITION].to_str().unwrap().to_owned();
    assert!(disposition.starts_with("attachment; filename=\"lykos-export-bob-"), "{disposition}");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let export: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(export["profile"]["username"], "bob");
    assert!(export["profile"].get("password_hash").is_none());
    assert_eq!(export["sessions"].as_array().unwrap().len(), 1);
    assert!(export["sessions"][0].get("token_hash").is_none());
    let memberships = export["memberships"].as_array().unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0]["community_name"], "Rudel");
    assert_eq!(memberships[0]["nickname"], "Bobby");
    let messages = export["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "Hallo Rudel");
    assert_eq!(messages[0]["channel_name"], "allgemein");
    assert_eq!(messages[0]["community_name"], "Rudel");

    let (status, _) = app.call("GET", "/auth/me/export", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn delete_account(db: PgPool) {
    let p = Profiles::new(db);
    let app = &p.app;
    let (alex, bob, carol, rudel) = setup(app).await;
    let bob_id = app.user_id(&bob).await;
    let messages_url = format!("/channels/{}/messages", rudel.text_channel);
    app.call("POST", &messages_url, Some(&bob), Some(json!({ "content": "bleibt" }))).await;
    p.upload_avatar(&bob, &png(64, 64)).await;
    assert_eq!(p.stored_keys().await.len(), 1);

    // Falsches Passwort.
    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&bob), Some(json!({ "password": "falsch-falsch-falsch" }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "wrong_password"));

    // Besitzer und letzter Instanz-Admin: 409.
    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&alex), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "owns_communities"));

    // Bob löschen, Nachrichten bleiben als „Gelöschter Nutzer“.
    let (status, _) = app.call("DELETE", "/auth/me", Some(&bob), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/auth/me", Some(&bob), None).await.0, StatusCode::UNAUTHORIZED);
    let (_, msgs) = app.call("GET", &messages_url, Some(&alex), None).await;
    assert_eq!(msgs[0]["content"], "bleibt");
    assert!(msgs[0]["author"].is_null());
    let (_, members) =
        app.call("GET", &format!("/communities/{}/members", rudel.id), Some(&alex), None).await;
    assert!(!members.as_array().unwrap().iter().any(|m| m["user"]["id"] == bob_id.as_str()));
    assert!(p.stored_keys().await.is_empty(), "Avatar-Datei gelöscht");
    // Benutzername ist wieder frei (Einladung nötig).
    let rudel_invite = {
        let (_, list) = app.call("GET", "/communities", Some(&alex), None).await;
        let id = list.as_array().unwrap().iter().find(|c| c["name"] == "Rudel").unwrap()["id"].clone();
        let (_, inv) =
            app.call("POST", &format!("/communities/{}/invites", s(&id)), Some(&alex), Some(json!({}))).await;
        s(&inv["code"]).to_owned()
    };
    let dave = app.user("bob", Some(&rudel_invite)).await;

    // Mit Nachrichten löschen.
    app.call("POST", &messages_url, Some(&dave), Some(json!({ "content": "weg damit" }))).await;
    let (status, _) = app
        .call(
            "DELETE",
            "/auth/me",
            Some(&dave),
            Some(json!({ "password": PASSWORD, "delete_messages": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, msgs) = app.call("GET", &messages_url, Some(&alex), None).await;
    assert!(!msgs.as_array().unwrap().iter().any(|m| m["content"] == "weg damit"));
    assert!(msgs.as_array().unwrap().iter().any(|m| m["content"] == "bleibt"));

    // Carol löscht sich; Alex (Instanz-Admin) ohne Communities wäre der letzte Admin.
    let (status, _) =
        app.call("DELETE", "/auth/me", Some(&carol), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for c in ["Rudel", "Andere"] {
        let (_, list) = app.call("GET", "/communities", Some(&alex), None).await;
        let id = list.as_array().unwrap().iter().find(|x| x["name"] == c).unwrap()["id"].clone();
        assert_eq!(
            app.call("DELETE", &format!("/communities/{}", s(&id)), Some(&alex), None).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&alex), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "last_instance_admin"));
}
