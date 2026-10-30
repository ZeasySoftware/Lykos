//! Integrationstests für Rollen-Reihenfolge und Rollenhierarchie (Paket E).

mod common;

use axum::http::StatusCode;
use common::{TestApp, TestCommunity, s};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn create_role(
    app: &TestApp,
    token: &str,
    community: &TestCommunity,
    name: &str,
    perms: i64,
) -> String {
    let (status, role) = app
        .call(
            "POST",
            &format!("/communities/{}/roles", community.id),
            Some(token),
            Some(json!({ "name": name, "permissions": perms })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    s(&role["id"]).to_owned()
}

/// Nicht-Standardrollen in Hierarchie-Reihenfolge (höchste zuerst).
async fn order(app: &TestApp, token: &str, community: &TestCommunity) -> Vec<String> {
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(token), None).await;
    let mut roles: Vec<&Value> =
        detail["roles"].as_array().unwrap().iter().filter(|r| r["is_default"] == false).collect();
    roles.sort_by_key(|r| r["position"].as_i64().unwrap());
    roles.iter().map(|r| s(&r["id"]).to_owned()).collect()
}

async fn reorder(app: &TestApp, token: &str, community: &TestCommunity, ids: &[&str]) -> (StatusCode, Value) {
    app.call(
        "PATCH",
        &format!("/communities/{}/roles/order", community.id),
        Some(token),
        Some(json!({ "role_ids": ids })),
    )
    .await
}

const MANAGE_ROLES: i64 = 1 << 9;
const SEND_MESSAGES: i64 = 1 << 1;

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn owner_reorders_roles_in_bulk(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let a = create_role(&app, &alex, &community, "A", 0).await;
    let b = create_role(&app, &alex, &community, "B", 0).await;
    let c = create_role(&app, &alex, &community, "C", 0).await;
    assert_eq!(order(&app, &alex, &community).await, vec![a.clone(), b.clone(), c.clone()]);

    let (status, roles) = reorder(&app, &alex, &community, &[&c, &a, &b]).await;
    assert_eq!(status, StatusCode::OK, "{roles}");
    assert_eq!(roles.as_array().unwrap().len(), 4); // inkl. Standardrolle
    assert_eq!(order(&app, &alex, &community).await, vec![c.clone(), a.clone(), b.clone()]);

    // Positionen sind danach lückenlos 1..n.
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    let mut positions: Vec<i64> = detail["roles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["is_default"] == false)
        .map(|r| r["position"].as_i64().unwrap())
        .collect();
    positions.sort();
    assert_eq!(positions, vec![1, 2, 3]);

    // Audit-Eintrag.
    let (_, log) =
        app.call("GET", &format!("/communities/{}/audit-log", community.id), Some(&alex), None).await;
    assert!(log.as_array().unwrap().iter().any(|e| e["action"] == "role.reorder"));
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn reorder_rejects_invalid_lists(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let a = create_role(&app, &alex, &community, "A", 0).await;
    let b = create_role(&app, &alex, &community, "B", 0).await;

    // Unvollständig, doppelt, mit Standardrolle, fremde ID.
    for ids in [
        vec![a.as_str()],
        vec![a.as_str(), a.as_str()],
        vec![a.as_str(), b.as_str(), community.default_role.as_str()],
        vec![a.as_str(), "0190f000-0000-7000-8000-000000000000"],
    ] {
        let (status, body) = reorder(&app, &alex, &community, &ids).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{ids:?}");
        assert_eq!(body["error"], "invalid_request");
    }

    // Ohne MANAGE_ROLES: verboten.
    let (status, _) = reorder(&app, &bob, &community, &[&b, &a]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Standardrolle hat keine Position.
    let (status, _) = app
        .call(
            "PATCH",
            &format!("/roles/{}", community.default_role),
            Some(&alex),
            Some(json!({ "position": 3 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn hierarchy_limits_non_owners(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carl = app.invited_user(&alex, &community, "carl").await;
    let bob_id = app.user_id(&bob).await;
    let carl_id = app.user_id(&carl).await;

    // Reihenfolge: Admins (1) > Mods (2) > Helfer (3) > Gäste (4).
    let admins = create_role(&app, &alex, &community, "Admins", MANAGE_ROLES | SEND_MESSAGES).await;
    let mods = create_role(&app, &alex, &community, "Mods", MANAGE_ROLES | SEND_MESSAGES).await;
    let helfer = create_role(&app, &alex, &community, "Helfer", SEND_MESSAGES).await;
    let gaeste = create_role(&app, &alex, &community, "Gäste", 0).await;
    let assign = |token: &str, user: &str, role: &str, method: &'static str| {
        let path = format!("/communities/{}/members/{user}/roles/{role}", community.id);
        let token = token.to_owned();
        let app = &app;
        async move { app.call(method, &path, Some(&token), None).await.0 }
    };
    assert_eq!(assign(&alex, &bob_id, &mods, "PUT").await, StatusCode::NO_CONTENT);

    // Bob (Mods) darf Rollen unterhalb verwalten und zuweisen …
    assert_eq!(assign(&bob, &carl_id, &helfer, "PUT").await, StatusCode::NO_CONTENT);
    assert_eq!(assign(&bob, &carl_id, &helfer, "DELETE").await, StatusCode::NO_CONTENT);
    let (status, _) =
        app.call("PATCH", &format!("/roles/{gaeste}"), Some(&bob), Some(json!({ "name": "Besucher" }))).await;
    assert_eq!(status, StatusCode::OK);

    // … aber weder die eigene noch höhere Rollen zuweisen, bearbeiten oder löschen.
    for role in [&mods, &admins] {
        assert_eq!(assign(&bob, &carl_id, role, "PUT").await, StatusCode::FORBIDDEN);
        let (status, _) =
            app.call("PATCH", &format!("/roles/{role}"), Some(&bob), Some(json!({ "name": "x" }))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = app.call("DELETE", &format!("/roles/{role}"), Some(&bob), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // Verschieben: nicht über oder auf die eigene Position.
    for position in [1, 2] {
        let (status, _) = app
            .call("PATCH", &format!("/roles/{gaeste}"), Some(&bob), Some(json!({ "position": position })))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "Position {position}");
    }
    let (status, _) =
        app.call("PATCH", &format!("/roles/{mods}"), Some(&bob), Some(json!({ "position": 5 }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Bulk: Rollen unterhalb dürfen getauscht werden …
    let (status, _) = reorder(&app, &bob, &community, &[&admins, &mods, &gaeste, &helfer]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        order(&app, &alex, &community).await,
        vec![admins.clone(), mods.clone(), gaeste.clone(), helfer.clone()]
    );
    // … aber nichts über die eigene Rolle hinaus und die eigene Rolle nicht nach unten.
    let (status, _) = reorder(&app, &bob, &community, &[&admins, &gaeste, &mods, &helfer]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = reorder(&app, &bob, &community, &[&mods, &admins, &gaeste, &helfer]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        order(&app, &alex, &community).await,
        vec![admins.clone(), mods.clone(), gaeste.clone(), helfer.clone()]
    );

    // Neue Rollen landen ganz unten und sind damit für Bob verwaltbar.
    let neu = create_role(&app, &bob, &community, "Neu", SEND_MESSAGES).await;
    assert_eq!(order(&app, &alex, &community).await.last(), Some(&neu));
    let (status, _) = app.call("DELETE", &format!("/roles/{neu}"), Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Der Besitzer ist an keine Hierarchie gebunden.
    let (status, _) = reorder(&app, &alex, &community, &[&helfer, &gaeste, &mods, &admins]).await;
    assert_eq!(status, StatusCode::OK);
    // Jetzt steht Mods über Admins → Bob darf Admins zuweisen.
    assert_eq!(assign(&bob, &carl_id, &admins, "PUT").await, StatusCode::NO_CONTENT);
}

const MANAGE_NICKNAMES: i64 = 1 << 22;

/// Fremde Nicknames (`MANAGE_NICKNAMES`) folgen der Rollenhierarchie.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn nicknames_follow_hierarchy(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carl = app.invited_user(&alex, &community, "carl").await;
    let dora = app.invited_user(&alex, &community, "dora").await;
    let erik = app.invited_user(&alex, &community, "erik").await;
    let [bob_id, carl_id, dora_id, erik_id] = [
        app.user_id(&bob).await,
        app.user_id(&carl).await,
        app.user_id(&dora).await,
        app.user_id(&erik).await,
    ];

    // Admins (1) > Mods (2) > Helfer (3); Bob und Carl sind Mods, Dora Admin, Erik Helfer.
    let admins = create_role(&app, &alex, &community, "Admins", MANAGE_NICKNAMES).await;
    let mods = create_role(&app, &alex, &community, "Mods", MANAGE_NICKNAMES).await;
    let helfer = create_role(&app, &alex, &community, "Helfer", 0).await;
    for (user, role) in [(&bob_id, &mods), (&carl_id, &mods), (&dora_id, &admins), (&erik_id, &helfer)] {
        let path = format!("/communities/{}/members/{user}/roles/{role}", community.id);
        assert_eq!(app.call("PUT", &path, Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    }
    let nick = |token: &str, target: &str| {
        let path = format!("/communities/{}/members/{target}", community.id);
        let token = token.to_owned();
        let app = &app;
        async move { app.call("PATCH", &path, Some(&token), Some(json!({ "nickname": "Neu" }))).await.0 }
    };

    // Bob (Mods): darunter ja (Helfer, nur Standardrolle), gleich hoch oder höher nein.
    assert_eq!(nick(&bob, &erik_id).await, StatusCode::OK);
    let fresh = app.invited_user(&alex, &community, "frida").await;
    let fresh_id = app.user_id(&fresh).await;
    assert_eq!(nick(&bob, &fresh_id).await, StatusCode::OK);
    assert_eq!(nick(&bob, &carl_id).await, StatusCode::FORBIDDEN);
    assert_eq!(nick(&bob, &dora_id).await, StatusCode::FORBIDDEN);
    // Dora (Admins) darf Mods umbenennen, der Besitzer alle.
    assert_eq!(nick(&dora, &bob_id).await, StatusCode::OK);
    assert_eq!(nick(&alex, &dora_id).await, StatusCode::OK);
    // Eigener Nickname bleibt unabhängig von der Hierarchie.
    assert_eq!(nick(&bob, "@me").await, StatusCode::OK);
}

const KICK_MEMBERS: i64 = 1 << 10;

/// Rauswerfen (`KICK_MEMBERS`) folgt der Rollenhierarchie.
#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn kick_follows_hierarchy(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let bob = app.invited_user(&alex, &community, "bob").await;
    let carl = app.invited_user(&alex, &community, "carl").await;
    let dora = app.invited_user(&alex, &community, "dora").await;
    let erik = app.invited_user(&alex, &community, "erik").await;
    let [bob_id, carl_id, dora_id, erik_id] = [
        app.user_id(&bob).await,
        app.user_id(&carl).await,
        app.user_id(&dora).await,
        app.user_id(&erik).await,
    ];

    // Admins (1) > Mods (2); Bob und Carl sind Mods, Dora Admin, Erik nur Standardrolle.
    let admins = create_role(&app, &alex, &community, "Admins", KICK_MEMBERS).await;
    let mods = create_role(&app, &alex, &community, "Mods", KICK_MEMBERS).await;
    for (user, role) in [(&bob_id, &mods), (&carl_id, &mods), (&dora_id, &admins)] {
        let path = format!("/communities/{}/members/{user}/roles/{role}", community.id);
        assert_eq!(app.call("PUT", &path, Some(&alex), None).await.0, StatusCode::NO_CONTENT);
    }
    let kick = |token: &str, target: &str| {
        let path = format!("/communities/{}/members/{target}", community.id);
        let token = token.to_owned();
        let app = &app;
        async move { app.call("DELETE", &path, Some(&token), None).await.0 }
    };

    // Bob (Mods) kann weder gleich hohe noch höhere Mitglieder rauswerfen, wohl aber darunter.
    assert_eq!(kick(&bob, &carl_id).await, StatusCode::FORBIDDEN);
    assert_eq!(kick(&bob, &dora_id).await, StatusCode::FORBIDDEN);
    assert_eq!(kick(&bob, &erik_id).await, StatusCode::NO_CONTENT);
    // Dora (Admins) wirft Mods raus, der Besitzer jeden.
    assert_eq!(kick(&dora, &carl_id).await, StatusCode::NO_CONTENT);
    assert_eq!(kick(&alex, &dora_id).await, StatusCode::NO_CONTENT);
    // Verlassen bleibt unabhängig von der Hierarchie.
    assert_eq!(kick(&bob, &bob_id).await, StatusCode::NO_CONTENT);
}
