//! Integrationstests für Fehler- und Vorschlagsmeldungen (Paket K): Melden, Sichtbarkeit, Rate-Limit,
//! Stimmen, Admin-Verwaltung mit Gateway-Event, Screenshots und die Gitea-Weiterleitung (Mock-Server).

mod common;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    routing::{get, post},
};
use common::{TestApp, s};
use http_body_util::BodyExt;
use lykos_server::{
    attachments,
    config::{Config, FeedbackGiteaConfig},
    storage::{LocalStorage, Storage},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;

const BOUNDARY: &str = "lykos-feedback-boundary";

/// Minimaler PNG-Kopf mit den angegebenen Maßen.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0");
    v.extend_from_slice(&[0u8; 64]);
    v
}

/// Testumgebung: Alex (erster Nutzer = Instanz-Admin), Bob und Carol (normale Nutzer).
struct Env {
    app: TestApp,
    dir: TempDir,
    events: lykos_server::events::EventBus,
    alex: String,
    bob: String,
    carol: String,
}

impl Env {
    async fn new(db: PgPool) -> Self {
        Self::with_config(db, Config::for_tests()).await
    }

    async fn with_config(db: PgPool, mut config: Config) -> Self {
        let dir = tempfile::tempdir().unwrap();
        config.storage_dir = dir.path().to_path_buf();
        let state = lykos_server::state(db.clone(), config);
        let events = state.events.clone();
        let app = TestApp { router: lykos_server::app(state).unwrap(), db };
        let alex = app.user("alex", None).await;
        let rudel = app.community(&alex, "Rudel").await;
        let bob = app.invited_user(&alex, &rudel, "bob").await;
        let carol = app.invited_user(&alex, &rudel, "carol").await;
        Self { app, dir, events, alex, bob, carol }
    }

    async fn report(&self, token: &str, kind: &str, title: &str) -> Value {
        let (status, body) = self
            .app
            .call(
                "POST",
                "/feedback",
                Some(token),
                Some(json!({ "kind": kind, "title": title, "description": "Beschreibung **mit** Markdown" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    async fn upload_screenshot(&self, token: &str, id: &str, data: &[u8]) -> (StatusCode, Value) {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"bild.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/feedback/{id}/screenshot"))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        self.app.send(req).await
    }

    async fn raw_get(&self, token: &str, path: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        let req = Request::builder()
            .uri(format!("/api{path}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = self.app.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn stored_keys(&self) -> Vec<String> {
        LocalStorage::new(self.dir.path()).list().await.unwrap().into_iter().map(|o| o.key).collect()
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn report_is_validated_and_tech_info_cleaned(db: PgPool) {
    let env = Env::new(db).await;

    for (body, code) in [
        (json!({ "kind": "feature", "title": "Titel", "description": "x" }), "invalid_kind"),
        (json!({ "kind": "bug", "title": "ab", "description": "x" }), "invalid_title"),
        (json!({ "kind": "bug", "title": "Titel", "description": "   " }), "invalid_description"),
        (
            json!({ "kind": "bug", "title": "Titel", "description": "x".repeat(10_001) }),
            "invalid_description",
        ),
        (
            json!({ "kind": "bug", "title": "Titel", "description": "x", "tech_info": [1] }),
            "invalid_tech_info",
        ),
    ] {
        let (status, res) = env.app.call("POST", "/feedback", Some(&env.bob), Some(body)).await;
        assert_eq!((status, res["error"].as_str()), (StatusCode::BAD_REQUEST, Some(code)));
    }
    let (status, _) = env
        .app
        .call("POST", "/feedback", None, Some(json!({ "kind": "bug", "title": "Titel", "description": "x" })))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, created) = env
        .app
        .call(
            "POST",
            "/feedback",
            Some(&env.bob),
            Some(json!({
                "kind": "bug",
                "title": "  Absturz\nbeim Senden ",
                "description": "Passiert immer.",
                "tech_info": {
                    "app_version": "0.3.0",
                    "platform": "web",
                    "os": "Windows",
                    "window": { "width": 1280, "height": 720 },
                    "view": "/channels/0192f3a4-5b6c-7d8e-9f01-23456789abcd",
                    "errors": [{ "at": "2026-10-29T12:00:00Z", "message": "Bearer abcdefabcdef1234567890abcdef failed" }],
                    "token": "geheim",
                },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["title"], "Absturz beim Senden");
    assert_eq!(created["status"], "neu");
    assert_eq!(created["mine"], true);
    assert!(created["number"].as_i64().unwrap() >= 1);
    assert_eq!(created["screenshot_url"], Value::Null);
    let info = &created["tech_info"];
    assert_eq!(info["view"], "/channels/<id>");
    assert_eq!(info["errors"][0]["message"], "Bearer <redacted> failed");
    assert!(info.get("token").is_none());
    // Melder sieht weder sich selbst als `reporter` noch den Issue-Link (nur Admins).
    assert!(created.get("reporter").is_none());
    assert!(created.get("issue_url").is_none());

    // Ohne Opt-in: keine technischen Infos.
    let plain = env.report(&env.bob, "other", "Nur eine Frage").await;
    assert_eq!(plain["tech_info"], Value::Null);
    let second = plain["number"].as_i64().unwrap();
    assert_eq!(second, created["number"].as_i64().unwrap() + 1);

    let (_, mine) = env.app.call("GET", "/feedback/mine", Some(&env.bob), None).await;
    let titles: Vec<&str> = mine.as_array().unwrap().iter().map(|f| s(&f["title"])).collect();
    assert_eq!(titles, ["Nur eine Frage", "Absturz beim Senden"]);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn bug_reports_are_private_suggestions_are_public(db: PgPool) {
    let env = Env::new(db).await;
    let bug = env.report(&env.bob, "bug", "Bobs Fehler").await;
    let other = env.report(&env.bob, "other", "Bobs Sonstiges").await;
    let idea = env.report(&env.bob, "suggestion", "Dunkles Design").await;

    // Carol sieht Bobs Fehlerbericht und Sonstiges nicht – weder direkt noch in Listen.
    for f in [&bug, &other] {
        let (status, _) =
            env.app.call("GET", &format!("/feedback/{}", s(&f["id"])), Some(&env.carol), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    let (_, carol_mine) = env.app.call("GET", "/feedback/mine", Some(&env.carol), None).await;
    assert_eq!(carol_mine, json!([]));
    let (status, list) = env.app.call("GET", "/feedback/suggestions", Some(&env.carol), None).await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1);
    // Öffentliche Ansicht: ohne Melder, technische Infos und Screenshot.
    let public = &list[0];
    assert_eq!(public["title"], "Dunkles Design");
    assert_eq!(public["mine"], false);
    for hidden in ["reporter", "tech_info", "screenshot_url", "issue_url"] {
        assert!(public.get(hidden).is_none(), "{hidden} sichtbar: {public}");
    }
    let (status, _) =
        env.app.call("GET", &format!("/feedback/{}", s(&idea["id"])), Some(&env.carol), None).await;
    assert_eq!(status, StatusCode::OK);

    // Nicht-Admins kommen nicht an die Verwaltung.
    for (method, path) in [
        ("GET", "/admin/feedback".to_owned()),
        ("GET", "/admin/feedback/settings".to_owned()),
        ("GET", format!("/admin/feedback/{}", s(&bug["id"]))),
        ("DELETE", format!("/admin/feedback/{}", s(&bug["id"]))),
        ("POST", format!("/admin/feedback/{}/issue", s(&bug["id"]))),
    ] {
        let (status, _) = env.app.call(method, &path, Some(&env.bob), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }
    let (status, _) = env
        .app
        .call(
            "PATCH",
            &format!("/admin/feedback/{}", s(&bug["id"])),
            Some(&env.carol),
            Some(json!({ "status": "erledigt" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin sieht alles, mit Melder; Filter nach Typ und Status.
    let (status, all) = env.app.call("GET", "/admin/feedback", Some(&env.alex), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all.as_array().unwrap().len(), 3);
    assert_eq!(all[0]["reporter"]["username"], "bob");
    let (_, bugs) = env.app.call("GET", "/admin/feedback?kind=bug&status=neu", Some(&env.alex), None).await;
    assert_eq!(bugs.as_array().unwrap().len(), 1);
    assert_eq!(bugs[0]["id"], bug["id"]);
    let (_, none) = env.app.call("GET", "/admin/feedback?status=erledigt", Some(&env.alex), None).await;
    assert_eq!(none, json!([]));
    let (status, _) = env.app.call("GET", "/admin/feedback?status=offen", Some(&env.alex), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, settings) = env.app.call("GET", "/admin/feedback/settings", Some(&env.alex), None).await;
    assert_eq!((status, settings), (StatusCode::OK, json!({ "gitea": false })));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reports_are_rate_limited_per_user(db: PgPool) {
    let env = Env::new(db).await;
    for i in 0..10 {
        env.report(&env.bob, "bug", &format!("Fehler {i}")).await;
    }
    let req = Request::builder()
        .method("POST")
        .uri("/api/feedback")
        .header(header::AUTHORIZATION, format!("Bearer {}", env.bob))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "kind": "bug", "title": "Elfter", "description": "x" }).to_string()))
        .unwrap();
    let res = env.app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = res.headers()[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((1..=3600).contains(&retry), "{retry}");

    // Andere Nutzer sind nicht betroffen.
    env.report(&env.carol, "bug", "Carols Fehler").await;
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn suggestions_can_be_voted_once(db: PgPool) {
    let env = Env::new(db).await;
    let idea = env.report(&env.bob, "suggestion", "Mehr Emojis").await;
    let newer = env.report(&env.bob, "suggestion", "Themes").await;
    let bug = env.report(&env.bob, "bug", "Fehler").await;
    let vote = |id: &Value| format!("/feedback/{}/vote", s(id));

    let (status, res) = env.app.call("PUT", &vote(&idea["id"]), Some(&env.carol), None).await;
    assert_eq!((status, res), (StatusCode::OK, json!({ "vote_count": 1, "voted": true })));
    // Doppelte Stimme → 409, Zähler bleibt.
    let (status, res) = env.app.call("PUT", &vote(&idea["id"]), Some(&env.carol), None).await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::CONFLICT, Some("already_voted")));
    env.app.call("PUT", &vote(&idea["id"]), Some(&env.alex), None).await;

    // Sortierung nach Stimmen bzw. neu.
    let (_, by_votes) = env.app.call("GET", "/feedback/suggestions?sort=votes", Some(&env.carol), None).await;
    assert_eq!(by_votes[0]["id"], idea["id"]);
    assert_eq!(by_votes[0]["vote_count"], 2);
    assert_eq!(by_votes[0]["voted"], true);
    let (_, by_new) = env.app.call("GET", "/feedback/suggestions?sort=new", Some(&env.carol), None).await;
    assert_eq!(by_new[0]["id"], newer["id"]);
    let (status, _) = env.app.call("GET", "/feedback/suggestions?sort=zufall", Some(&env.carol), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Zurücknehmen ist idempotent.
    for _ in 0..2 {
        let (status, res) = env.app.call("DELETE", &vote(&idea["id"]), Some(&env.carol), None).await;
        assert_eq!((status, res), (StatusCode::OK, json!({ "vote_count": 1, "voted": false })));
    }

    // Fehlerberichte: fremde nicht sichtbar (404), eigene sind keine Vorschläge (400).
    let (status, _) = env.app.call("PUT", &vote(&bug["id"]), Some(&env.carol), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, res) = env.app.call("PUT", &vote(&bug["id"]), Some(&env.bob), None).await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::BAD_REQUEST, Some("not_a_suggestion")));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn admin_status_and_replies_reach_the_reporter(db: PgPool) {
    let env = Env::new(db).await;
    let bug = env.report(&env.bob, "bug", "Login hängt").await;
    let id = s(&bug["id"]).to_owned();
    let bob_id = env.app.user_id(&env.bob).await;
    let mut rx = env.events.subscribe();

    let (status, updated) = env
        .app
        .call(
            "PATCH",
            &format!("/admin/feedback/{id}"),
            Some(&env.alex),
            Some(json!({ "status": "in_arbeit" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["status"], "in_arbeit");
    let (status, _) = env
        .app
        .call("PATCH", &format!("/admin/feedback/{id}"), Some(&env.alex), Some(json!({ "status": "fertig" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Interne Notiz und Antwort.
    let (status, _) = env
        .app
        .call(
            "POST",
            &format!("/admin/feedback/{id}/replies"),
            Some(&env.alex),
            Some(json!({ "body": "Vermutlich der Proxy", "internal": true })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, detail) = env
        .app
        .call(
            "POST",
            &format!("/admin/feedback/{id}/replies"),
            Some(&env.alex),
            Some(json!({ "body": "Danke, ist behoben!" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(detail["replies"].as_array().unwrap().len(), 2);
    let (status, _) = env
        .app
        .call("POST", &format!("/admin/feedback/{id}/replies"), Some(&env.alex), Some(json!({ "body": " " })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Gateway: FEEDBACK_UPDATE an Bob (Statuswechsel und öffentliche Antwort, nicht die interne Notiz).
    let mut updates = vec![];
    while let Ok(ev) = rx.try_recv() {
        if ev.event.kind == "FEEDBACK_UPDATE" {
            assert_eq!(ev.scope, lykos_server::events::Scope::User(bob_id.parse().unwrap()));
            updates.push(ev.event.data.clone());
        }
    }
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0]["status"], "in_arbeit");
    assert_eq!(updates[1]["replies"].as_array().unwrap().len(), 1);
    assert!(updates[1].get("reporter").is_none());

    // Bob sieht nur die Antwort, nicht die interne Notiz.
    let (_, mine) = env.app.call("GET", "/feedback/mine", Some(&env.bob), None).await;
    let replies = mine[0]["replies"].as_array().unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["body"], "Danke, ist behoben!");
    assert_eq!(replies[0]["author"]["username"], "alex");

    // Admin löscht → FEEDBACK_DELETE an Bob, danach 404.
    let (status, _) = env.app.call("DELETE", &format!("/admin/feedback/{id}"), Some(&env.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted = std::iter::from_fn(|| rx.try_recv().ok()).find(|ev| ev.event.kind == "FEEDBACK_DELETE");
    assert_eq!(deleted.unwrap().event.data, json!({ "id": id }));
    let (status, _) = env.app.call("GET", &format!("/feedback/{id}"), Some(&env.bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn screenshots_are_private_and_cleaned_up(db: PgPool) {
    let env = Env::new(db).await;
    let bug = env.report(&env.bob, "bug", "Darstellungsfehler").await;
    let id = s(&bug["id"]).to_owned();

    // Nur der Melder darf anhängen; nur Bilder.
    let (status, _) = env.upload_screenshot(&env.carol, &id, &png(800, 600)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, res) = env.upload_screenshot(&env.bob, &id, b"kein bild").await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_image")));
    let (status, res) = env.upload_screenshot(&env.bob, &id, &png(800, 600)).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let url = s(&res["screenshot_url"]).to_owned();
    assert_eq!(url, format!("/api/feedback/{id}/screenshot"));
    let (status, res) = env.upload_screenshot(&env.bob, &id, &png(10, 10)).await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::CONFLICT, Some("screenshot_exists")));
    let path = url.trim_start_matches("/api");

    for token in [&env.bob, &env.alex] {
        let (status, headers, bytes) = env.raw_get(token, path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(bytes, png(800, 600));
    }
    assert_eq!(env.raw_get(&env.carol, path).await.0, StatusCode::NOT_FOUND);

    // Screenshot eines Vorschlags bleibt für andere unsichtbar.
    let idea = env.report(&env.bob, "suggestion", "Idee mit Bild").await;
    let idea_id = s(&idea["id"]).to_owned();
    env.upload_screenshot(&env.bob, &idea_id, &png(20, 20)).await;
    assert_eq!(
        env.raw_get(&env.carol, &format!("/feedback/{idea_id}/screenshot")).await.0,
        StatusCode::NOT_FOUND
    );

    // Der Abgleich im Speicher hält Screenshots für bekannt.
    assert_eq!(env.stored_keys().await.len(), 2);
    attachments::cleanup(&env.app.db, &LocalStorage::new(env.dir.path()), std::time::Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(env.stored_keys().await.len(), 2);

    // Löschen (Admin bzw. Melder selbst) entfernt die Dateien.
    let (status, _) = env.app.call("DELETE", &format!("/admin/feedback/{id}"), Some(&env.alex), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(env.stored_keys().await.len(), 1);
    let (status, _) = env.app.call("DELETE", &format!("/feedback/{idea_id}"), Some(&env.carol), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env.app.call("DELETE", &format!("/feedback/{idea_id}"), Some(&env.bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(env.stored_keys().await.is_empty());
}

// --- Gitea --------------------------------------------------------------------

#[derive(Clone, Default)]
struct GiteaMock {
    /// Empfangene Issue-Anlagen: (Authorization-Header, Body).
    issues: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn mock_gitea(mock: GiteaMock) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    async fn labels() -> Json<Value> {
        Json(json!([
            { "id": 1, "name": "bug" },
            { "id": 2, "name": "enhancement" },
            { "id": 3, "name": "Feedback" },
            { "id": 4, "name": "wontfix" },
        ]))
    }
    async fn issues(State(m): State<GiteaMock>, headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
        let auth = headers[header::AUTHORIZATION].to_str().unwrap().to_owned();
        let mut list = m.issues.lock().unwrap();
        list.push((auth, body));
        let n = list.len();
        Json(json!({ "number": n, "html_url": format!("https://git.example/ZeasyAlex/Lykos/issues/{n}") }))
    }

    let router = Router::new()
        .route("/api/v1/repos/ZeasyAlex/Lykos/labels", get(labels))
        .route("/api/v1/repos/ZeasyAlex/Lykos/issues", post(issues))
        .with_state(mock);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("{base}/api/v1/repos/ZeasyAlex/Lykos")
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn admin_can_forward_report_to_gitea(db: PgPool) {
    let mock = GiteaMock::default();
    let api_url = mock_gitea(mock.clone()).await;
    let mut config = Config::for_tests();
    config.feedback_gitea = Some(FeedbackGiteaConfig { api_url, token: "gitea-token".into() });
    let env = Env::with_config(db, config).await;

    let (_, settings) = env.app.call("GET", "/admin/feedback/settings", Some(&env.alex), None).await;
    assert_eq!(settings, json!({ "gitea": true }));

    let (_, bug) = env
        .app
        .call(
            "POST",
            "/feedback",
            Some(&env.bob),
            Some(json!({
                "kind": "bug",
                "title": "Ton fehlt",
                "description": "Kein Ton im Sprachkanal.",
                "tech_info": { "app_version": "0.3.0", "platform": "desktop" },
            })),
        )
        .await;
    let id = s(&bug["id"]).to_owned();
    let number = bug["number"].as_i64().unwrap();

    // Nur Admins.
    let (status, _) =
        env.app.call("POST", &format!("/admin/feedback/{id}/issue"), Some(&env.bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(mock.issues.lock().unwrap().is_empty());

    let (status, detail) =
        env.app.call("POST", &format!("/admin/feedback/{id}/issue"), Some(&env.alex), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["issue_url"], "https://git.example/ZeasyAlex/Lykos/issues/1");

    {
        let issues = mock.issues.lock().unwrap();
        assert_eq!(issues.len(), 1);
        let (auth, body) = &issues[0];
        assert_eq!(auth, "token gitea-token");
        assert_eq!(body["title"], format!("[Feedback #{number}] Ton fehlt"));
        let text = s(&body["body"]);
        // Beschreibung als Codeblock (keine Bilder, Links oder @-Erwähnungen im Issue).
        assert!(text.starts_with("```text\nKein Ton im Sprachkanal.\n```\n"), "{text}");
        assert!(text.contains(&format!("#{number}")));
        assert!(text.contains("\"app_version\": \"0.3.0\""));
        // Kein Nutzername im Issue.
        assert!(!text.contains("bob"), "{text}");
        // Labels: „feedback“ (ohne Groß/Klein-Unterschied) und „bug“.
        let mut labels: Vec<i64> =
            body["labels"].as_array().unwrap().iter().map(|l| l.as_i64().unwrap()).collect();
        labels.sort();
        assert_eq!(labels, [1, 3]);
    }

    // Kein zweites Issue.
    let (status, res) =
        env.app.call("POST", &format!("/admin/feedback/{id}/issue"), Some(&env.alex), None).await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::CONFLICT, Some("issue_exists")));
    assert_eq!(mock.issues.lock().unwrap().len(), 1);

    // Vorschläge bekommen „enhancement“.
    let idea = env.report(&env.bob, "suggestion", "Themes").await;
    let (status, _) = env
        .app
        .call("POST", &format!("/admin/feedback/{}/issue", s(&idea["id"])), Some(&env.alex), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let mut labels: Vec<i64> = mock.issues.lock().unwrap()[1].1["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_i64().unwrap())
        .collect();
    labels.sort();
    assert_eq!(labels, [2, 3]);

    // Der Melder sieht den Issue-Link nicht.
    let (_, mine) = env.app.call("GET", &format!("/feedback/{id}"), Some(&env.bob), None).await;
    assert!(mine.get("issue_url").is_none());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn gitea_forwarding_is_off_without_config_or_when_unreachable(db: PgPool) {
    let env = Env::new(db.clone()).await;
    let bug = env.report(&env.bob, "bug", "Fehler").await;
    let (status, res) = env
        .app
        .call("POST", &format!("/admin/feedback/{}/issue", s(&bug["id"])), Some(&env.alex), None)
        .await;
    assert_eq!(
        (status, res["error"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("gitea_not_configured"))
    );

    // Gitea nicht erreichbar → 503, kein Link gespeichert.
    let mut config = Config::for_tests();
    config.feedback_gitea = Some(FeedbackGiteaConfig {
        api_url: "http://127.0.0.1:9/api/v1/repos/a/b".into(),
        token: "t".into(),
    });
    let state = lykos_server::state(db, config);
    let app = TestApp { router: lykos_server::app(state).unwrap(), db: env.app.db.clone() };
    let (status, res) =
        app.call("POST", &format!("/admin/feedback/{}/issue", s(&bug["id"])), Some(&env.alex), None).await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("gitea_unavailable")));
    let (_, detail) =
        app.call("GET", &format!("/admin/feedback/{}", s(&bug["id"])), Some(&env.alex), None).await;
    assert_eq!(detail["issue_url"], Value::Null);
}
