//! Integrationstests für die Beitrittsrolle (Rolle, die neue Mitglieder automatisch bekommen).

mod common;

use axum::http::StatusCode;
use common::{TestApp, s};
use serde_json::{Value, json};
use sqlx::PgPool;

const MANAGE_COMMUNITY: i64 = 1 << 11;
const MANAGE_ROLES: i64 = 1 << 9;

/// Rollen-IDs eines Mitglieds laut Mitgliederliste.
async fn role_ids_of(app: &TestApp, token: &str, community_id: &str, user_id: &str) -> Vec<String> {
    let (status, members) =
        app.call("GET", &format!("/communities/{community_id}/members"), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let member = members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["user"]["id"] == user_id)
        .unwrap_or_else(|| panic!("{user_id} ist kein Mitglied: {members}"));
    member["role_ids"].as_array().unwrap().iter().map(|r| s(r).to_owned()).collect()
}

async fn create_with_join_role(app: &TestApp, token: &str, join_role: &str) -> Value {
    let (status, detail) = app
        .call(
            "POST",
            "/communities",
            Some(token),
            Some(json!({ "name": "Ausbildung", "join_role": join_role })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{detail}");
    detail
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn join_role_is_created_and_given_to_creator_and_new_members(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let alex_id = app.user_id(&alex).await;
    let detail = create_with_join_role(&app, &alex, "  Azubi ").await;
    let community_id = s(&detail["community"]["id"]).to_owned();

    // Rolle angelegt (Name getrimmt, ohne eigene Rechte, unterste Rolle) und als Beitrittsrolle gesetzt.
    let join_role_id = s(&detail["community"]["join_role_id"]).to_owned();
    let role = detail["roles"].as_array().unwrap().iter().find(|r| r["id"] == join_role_id.as_str()).unwrap();
    assert_eq!(role["name"], "Azubi");
    assert_eq!(role["permissions"], 0);
    assert_eq!(role["is_default"], false);

    // Ersteller hat sie.
    assert!(role_ids_of(&app, &alex, &community_id, &alex_id).await.contains(&join_role_id));

    // Neues Konto über Einladung.
    let (_, invite) =
        app.call("POST", &format!("/communities/{community_id}/invites"), Some(&alex), Some(json!({}))).await;
    let code = s(&invite["code"]).to_owned();
    let bob = app.user("bob", Some(&code)).await;
    let bob_id = app.user_id(&bob).await;
    assert_eq!(role_ids_of(&app, &alex, &community_id, &bob_id).await, vec![join_role_id.clone()]);

    // Bestehendes Konto (über eine andere Community registriert) nimmt die Einladung an.
    let other = app.community(&alex, "Andere").await;
    let carl = app.invited_user(&alex, &other, "carl").await;
    let carl_id = app.user_id(&carl).await;
    let (status, body) = app.call("POST", &format!("/invites/{code}/accept"), Some(&carl), None).await;
    assert!(status.is_success(), "{status} {body}");
    assert_eq!(role_ids_of(&app, &alex, &community_id, &carl_id).await, vec![join_role_id]);
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn without_join_role_nobody_gets_a_role(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let community = app.community(&alex, "Rudel").await;
    let (_, detail) = app.call("GET", &format!("/communities/{}", community.id), Some(&alex), None).await;
    assert!(detail["community"]["join_role_id"].is_null());
    let bob = app.invited_user(&alex, &community, "bob").await;
    let bob_id = app.user_id(&bob).await;
    assert!(role_ids_of(&app, &alex, &community.id, &bob_id).await.is_empty());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn invalid_join_role_name_is_rejected(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    for name in ["", "   ", &"x".repeat(65)] {
        let (status, body) = app
            .call("POST", "/communities", Some(&alex), Some(json!({ "name": "Rudel", "join_role": name })))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?}: {body}");
        assert_eq!(body["error"], "invalid_name");
    }
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn join_role_can_be_changed_removed_and_follows_role_deletion(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let detail = create_with_join_role(&app, &alex, "Mitglied").await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let path = format!("/communities/{community_id}");

    let (_, other) =
        app.call("POST", &format!("{path}/roles"), Some(&alex), Some(json!({ "name": "Gast" }))).await;
    let other_id = s(&other["id"]).to_owned();

    let (status, community) =
        app.call("PATCH", &path, Some(&alex), Some(json!({ "join_role_id": other_id }))).await;
    assert_eq!(status, StatusCode::OK, "{community}");
    assert_eq!(community["join_role_id"], other_id.as_str());

    // „@alle“ und fremde/unbekannte Rollen sind keine gültige Beitrittsrolle.
    let default_role =
        s(&detail["roles"].as_array().unwrap().iter().find(|r| r["is_default"] == true).unwrap()["id"])
            .to_owned();
    for bad in [default_role.as_str(), "0190a1b2-0000-7000-8000-000000000000"] {
        let (status, body) =
            app.call("PATCH", &path, Some(&alex), Some(json!({ "join_role_id": bad }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_role");
    }

    // Entfernen per null.
    let (_, community) = app.call("PATCH", &path, Some(&alex), Some(json!({ "join_role_id": null }))).await;
    assert!(community["join_role_id"].is_null());

    // Gelöschte Beitrittsrolle -> keine Beitrittsrolle mehr.
    app.call("PATCH", &path, Some(&alex), Some(json!({ "join_role_id": other_id }))).await;
    let (status, _) = app.call("DELETE", &format!("/roles/{other_id}"), Some(&alex), None).await;
    assert!(status.is_success());
    let (_, detail) = app.call("GET", &path, Some(&alex), None).await;
    assert!(detail["community"]["join_role_id"].is_null());
}

#[sqlx::test(migrator = "lykos_server::MIGRATOR")]
async fn changing_join_role_needs_manage_roles_and_hierarchy(db: PgPool) {
    let app = TestApp::new(db);
    let alex = app.user("alex", None).await;
    let detail = create_with_join_role(&app, &alex, "Mitglied").await;
    let community_id = s(&detail["community"]["id"]).to_owned();
    let path = format!("/communities/{community_id}");
    let role = |name: &str, perms: i64| {
        let (app, alex, path, name) = (&app, alex.clone(), path.clone(), name.to_owned());
        async move {
            let (_, r) = app
                .call(
                    "POST",
                    &format!("{path}/roles"),
                    Some(&alex),
                    Some(json!({ "name": name, "permissions": perms })),
                )
                .await;
            s(&r["id"]).to_owned()
        }
    };
    // Reihenfolge nach dem Anlegen: Mitglied (1) > Admins (2) > Verwalter (3) > Mods (4)
    let admins = role("Admins", 0).await;
    let verwalter = role("Verwalter", MANAGE_COMMUNITY).await;
    let mods = role("Mods", MANAGE_COMMUNITY | MANAGE_ROLES).await;
    let (status, _) = app
        .call(
            "PATCH",
            &format!("{path}/roles/order"),
            Some(&alex),
            Some(json!({ "role_ids": [admins, mods, verwalter, s(&detail["community"]["join_role_id"])] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, invite) = app.call("POST", &format!("{path}/invites"), Some(&alex), Some(json!({}))).await;
    let code = s(&invite["code"]).to_owned();
    let bob = app.user("bob", Some(&code)).await;
    let bob_id = app.user_id(&bob).await;
    let carl = app.user("carl", Some(&code)).await;
    let carl_id = app.user_id(&carl).await;
    let give = |user: String, role: String| {
        let (app, alex, path) = (&app, alex.clone(), path.clone());
        async move { app.call("PUT", &format!("{path}/members/{user}/roles/{role}"), Some(&alex), None).await.0 }
    };
    assert_eq!(give(bob_id, verwalter.clone()).await, StatusCode::NO_CONTENT);
    assert_eq!(give(carl_id, mods.clone()).await, StatusCode::NO_CONTENT);

    // Bob darf die Community verwalten, aber keine Rollen: Beitrittsrolle ändern ist verboten.
    let (status, _) = app.call("PATCH", &path, Some(&bob), Some(json!({ "join_role_id": null }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Carl (Mods) darf Rollen unter sich setzen, aber nicht „Admins“ über sich.
    let (status, _) = app.call("PATCH", &path, Some(&carl), Some(json!({ "join_role_id": verwalter }))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.call("PATCH", &path, Some(&carl), Some(json!({ "join_role_id": admins }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
