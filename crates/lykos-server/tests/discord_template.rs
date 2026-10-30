//! Import von Discord-Servervorlagen: Parser mit echter Vorlage (Fixture) und Anlegen über einen lokalen
//! Ersatz der Discord-API (kein Netz nötig).

mod common;

use std::net::SocketAddr;

use axum::{
    Router,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use common::{TestApp, s};
use lykos_server::{config::Config, discord_template, permissions};
use serde_json::{Value, json};
use sqlx::PgPool;

const FIXTURE: &str = include_str!("fixtures/discord_template_HqjFNRCT6EEU.json");
const CODE: &str = "HqjFNRCT6EEU";

/// Discord-API-Ersatz: liefert die Fixture unter `/api/v10/guilds/templates/HqjFNRCT6EEU`,
/// Müll unter `…/kaputt`, sonst 404 wie Discord.
async fn fake_discord() -> SocketAddr {
    async fn template(Path(code): Path<String>) -> Response {
        match code.as_str() {
            CODE => ([("content-type", "application/json")], FIXTURE).into_response(),
            "kaputt" => ([("content-type", "application/json")], "{\"code\": 1}").into_response(),
            _ => (StatusCode::NOT_FOUND, "{\"message\": \"Unknown Guild Template\", \"code\": 10057}")
                .into_response(),
        }
    }
    let router = Router::new().route("/api/v10/guilds/templates/{code}", get(template));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn app_with_discord(db: PgPool) -> TestApp {
    let addr = fake_discord().await;
    TestApp::with_config(
        db,
        Config { discord_api_url: format!("http://{addr}/api/v10"), ..Config::for_tests() },
    )
}

fn everyone_permissions() -> i64 {
    permissions::from_discord(2_222_085_186_768_449).0
}

#[test]
fn parser_maps_the_fixture() {
    let t = discord_template::parse(FIXTURE.as_bytes()).unwrap();
    assert_eq!(t.code, CODE);
    assert_eq!(t.name, "Test");
    assert_eq!(t.description.as_deref(), Some("ets"));
    assert_eq!(t.default_permissions.0, everyone_permissions());

    // 18 Rollen inkl. @everyone; höchste (letzte im Array) zuerst, lange Namen gekürzt.
    assert_eq!(t.roles.len(), 17);
    assert_eq!(t.roles[0].name, "Schwippschwager");
    assert_eq!(t.roles.last().unwrap().name, "Rematch");
    assert!(t.roles.iter().all(|r| (1..=64).contains(&r.name.chars().count())));
    assert!(
        t.roles
            .iter()
            .any(|r| r.name.starts_with("Gruppe mit einem sehr langen") && r.name.chars().count() == 64)
    );

    // 11 Kategorien, 27 Text- (inkl. Forum) und 13 Sprachkanäle.
    assert_eq!(t.categories.len(), 11);
    assert_eq!(t.categories[0].name, "📊 SERVER STATS 📊");
    assert_eq!(t.categories[1].name, "🎮 Gameserver");
    assert_eq!(t.channels.iter().filter(|c| c.kind == "text").count(), 27);
    assert_eq!(t.channels.iter().filter(|c| c.kind == "voice").count(), 13);
    assert_eq!(t.channels.iter().filter(|c| c.from_forum).count(), 1);
    assert!(t.channels.iter().all(|c| c.category.is_some()));

    // Je Kategorie: Text vor Sprache, Positionen aufsteigend.
    let positions: Vec<i32> = t.channels.iter().map(|c| c.position).collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]));
    for (i, _) in t.categories.iter().enumerate() {
        let kinds: Vec<&str> = t.channels.iter().filter(|c| c.category == Some(i)).map(|c| c.kind).collect();
        assert!(kinds.windows(2).all(|w| !(w[0] == "voice" && w[1] == "text")), "Kategorie {i}: {kinds:?}");
    }

    // Systemkanal = Kanal 2 („🛬︱78“), kein AFK-Kanal.
    assert_eq!(t.channels[t.system_channel.unwrap()].name, "🛬︱78");
    assert_eq!((t.afk_channel, t.afk_timeout), (None, Some(300)));

    // Overrides: Rematch-Kanal für @everyone versteckt, für die Rolle „Rematch“ sichtbar.
    let rematch = t.channels.iter().find(|c| c.name == "⚽︱rematch").unwrap();
    let everyone = rematch.overrides.iter().find(|o| o.role == discord_template::RoleRef::Default).unwrap();
    assert!(everyone.deny.contains(permissions::Permissions::VIEW_CHANNEL));
    assert!(
        rematch
            .overrides
            .iter()
            .any(|o| matches!(o.role, discord_template::RoleRef::Role(i) if t.roles[i].name == "Rematch")
                && o.allow.contains(permissions::Permissions::VIEW_CHANNEL))
    );

    // Forum-Hinweis, verworfene Rechte, Rollen-Optionen, Kategorie-Rechte.
    let notes = t.notes.join("\n");
    for needle in ["Forum", "verworfen", "erwähnbar", "Kategorien"] {
        assert!(notes.contains(needle), "Hinweis „{needle}“ fehlt: {notes}");
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn preview_shows_the_structure(db: PgPool) {
    let app = app_with_discord(db).await;
    let alex = app.user("alex", None).await;
    let (status, preview) =
        app.call("GET", &format!("/communities/templates/discord/{CODE}"), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["name"], "Test");
    assert_eq!(
        preview["counts"],
        json!({ "roles": 17, "categories": 11, "text_channels": 27, "voice_channels": 13 })
    );
    assert_eq!(preview["categories"].as_array().unwrap().len(), 11);
    assert_eq!(
        preview["categories"][1]["channels"][0],
        json!({ "name": "💡︱requests", "kind": "text", "from_forum": true })
    );
    assert_eq!(preview["channels"], json!([]));
    assert!(!preview["notes"].as_array().unwrap().is_empty());

    // Ohne Anmeldung nicht.
    let (status, _) = app.call("GET", &format!("/communities/templates/discord/{CODE}"), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn template_errors(db: PgPool) {
    let app = app_with_discord(db.clone()).await;
    let alex = app.user("alex", None).await;
    let preview = |code: &str| format!("/communities/templates/discord/{code}");
    let error = |body: &Value| body["error"].clone();

    let (status, body) = app.call("GET", &preview("Unbekannt1"), Some(&alex), None).await;
    assert_eq!((status, error(&body)), (StatusCode::NOT_FOUND, json!("template_not_found")));
    let (status, body) = app.call("GET", &preview("kaputt"), Some(&alex), None).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, json!("invalid_template")));
    let (status, body) = app.call("GET", &preview("x"), Some(&alex), None).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, json!("invalid_template_code")));

    // Fremde Hosts werden nie abgerufen.
    let (status, body) = app
        .call(
            "POST",
            "/communities",
            Some(&alex),
            Some(json!({ "discord_template": "https://evil.example/template/HqjFNRCT6EEU" })),
        )
        .await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, json!("invalid_template_code")));

    // Discord nicht erreichbar (Standard-Testkonfiguration zeigt ins Leere).
    let offline = TestApp::new(db);
    // Tokens liegen in der Datenbank – Alex ist auch in der zweiten App angemeldet.
    let (status, body) = offline.call("GET", &preview(CODE), Some(&alex), None).await;
    assert_eq!((status, error(&body)), (StatusCode::SERVICE_UNAVAILABLE, json!("template_fetch_failed")));

    // Nichts angelegt.
    let (_, list) = app.call("GET", "/communities", Some(&alex), None).await;
    assert_eq!(list, json!([]));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn template_fetches_are_rate_limited(db: PgPool) {
    let app = app_with_discord(db).await;
    let alex = app.user("alex", None).await;
    let mut last = StatusCode::OK;
    for _ in 0..25 {
        last = app.call("GET", &format!("/communities/templates/discord/{CODE}"), Some(&alex), None).await.0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn create_from_template(db: PgPool) {
    let app = app_with_discord(db).await;
    let alex = app.user("alex", None).await;
    let alex_id = app.user_id(&alex).await;
    let (status, detail) = app
        .call(
            "POST",
            "/communities",
            Some(&alex),
            Some(json!({ "name": "Bacanaks Lykos", "discord_template": format!("https://discord.new/{CODE}") })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{detail}");
    let community = &detail["community"];
    let community_id = s(&community["id"]).to_owned();
    assert_eq!(community["name"], "Bacanaks Lykos");
    assert_eq!(community["owner_id"], alex_id.as_str());
    assert_eq!(community["description"], "ets");

    // Rollen: Standardrolle mit den @everyone-Rechten, darüber 17 Rollen in Discord-Reihenfolge.
    let roles = detail["roles"].as_array().unwrap();
    assert_eq!(roles.len(), 18);
    let default = roles.iter().find(|r| r["is_default"] == true).unwrap();
    assert_eq!(default["permissions"], everyone_permissions());
    let top = roles.iter().find(|r| r["position"] == 1).unwrap();
    assert_eq!(top["name"], "Schwippschwager");
    let rematch_role = roles.iter().find(|r| r["name"] == "Rematch").unwrap();
    assert_eq!(rematch_role["position"], 17);
    assert_eq!(rematch_role["color"], 3_447_003);

    // Kategorien und Kanäle (Besitzer sieht alles).
    let categories = detail["categories"].as_array().unwrap();
    assert_eq!(categories.len(), 11);
    assert_eq!(categories[0]["name"], "📊 SERVER STATS 📊");
    let channels = detail["channels"].as_array().unwrap();
    assert_eq!(channels.iter().filter(|c| c["kind"] == "text").count(), 27);
    assert_eq!(channels.iter().filter(|c| c["kind"] == "voice").count(), 13);
    assert!(channels.iter().all(|c| !c["category_id"].is_null()));
    let by_name =
        |name: &str| channels.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("{name} fehlt"));
    let info = by_name("ℹ️︱info");
    assert_eq!(info["topic"], "Informationen zum Gameserver Management System");
    assert_eq!(info["category_id"], categories[1]["id"]);
    let privat = channels.iter().find(|c| c["name"] == "👀︱privat").unwrap();
    assert_eq!(privat["user_limit"], 2);
    assert_eq!(privat["audio_bitrate"], 64);

    // Systemkanal.
    assert_eq!(community["system_channel_id"], by_name("🛬︱78")["id"]);

    // Overrides: Rematch-Kanal für die Standardrolle versteckt, für die Rolle sichtbar.
    let rematch = by_name("⚽︱rematch");
    let (status, overrides) =
        app.call("GET", &format!("/channels/{}/overrides", s(&rematch["id"])), Some(&alex), None).await;
    assert_eq!(status, StatusCode::OK);
    let overrides = overrides.as_array().unwrap();
    let of = |role: &Value| overrides.iter().find(|o| o["role_id"] == role["id"]).cloned();
    assert_eq!(of(default).unwrap()["deny"].as_i64().unwrap() & 1, 1);
    assert_eq!(of(rematch_role).unwrap()["allow"].as_i64().unwrap() & 1, 1);

    // Ein Mitglied nur mit Standardrolle sieht private Bereiche (und deren Kategorien) nicht.
    let community_ref = common::TestCommunity {
        id: community_id.clone(),
        text_channel: String::new(),
        voice_channel: String::new(),
        default_role: s(&default["id"]).to_owned(),
    };
    let bob = app.invited_user(&alex, &community_ref, "bob").await;
    let (_, bob_detail) = app.call("GET", &format!("/communities/{community_id}"), Some(&bob), None).await;
    let bob_channels: Vec<&str> =
        bob_detail["channels"].as_array().unwrap().iter().map(|c| s(&c["name"])).collect();
    assert!(!bob_channels.contains(&"⚽︱rematch"));
    assert!(bob_channels.contains(&"🔊︱Fresskick"));
    let bob_categories: Vec<&str> =
        bob_detail["categories"].as_array().unwrap().iter().map(|c| s(&c["name"])).collect();
    assert!(bob_categories.contains(&"Marktplatte"));
    assert!(!bob_categories.contains(&"WoW"), "{bob_categories:?}");

    // Ohne Namen gilt der Name der Vorlage; Audit-Log nennt die Vorlage.
    let (status, second) =
        app.call("POST", "/communities", Some(&alex), Some(json!({ "discord_template": CODE }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["community"]["name"], "Test");
    let (_, log) =
        app.call("GET", &format!("/communities/{community_id}/audit-log"), Some(&alex), None).await;
    let create = log.as_array().unwrap().iter().find(|e| e["action"] == "community.create").unwrap();
    assert_eq!(create["data"]["discord_template"], CODE);
}
