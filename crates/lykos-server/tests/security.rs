//! Paket G: E-Mail-Adresse, Passwort vergessen, Zwei-Faktor-Authentifizierung.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{PASSWORD, TestApp, s};
use lykos_server::{config::Config, mail::CollectingMailer, mfa};
use serde_json::{Value, json};
use sqlx::PgPool;

const NEW_PASSWORD: &str = "ganz-neues-passwort";

/// App mit sammelndem Mailer und offener Registrierung (mehrere Nutzer ohne Einladung).
async fn app(db: PgPool) -> (TestApp, CollectingMailer) {
    open_registration(&db).await;
    TestApp::with_mailer(db, Config::for_tests())
}

async fn open_registration(db: &PgPool) {
    sqlx::query("UPDATE instance_settings SET value = '\"open\"' WHERE key = 'registration_mode'")
        .execute(db)
        .await
        .unwrap();
}

/// Token aus einem Link in der Mail (`…#token=…` – im Fragment, nicht in der Query).
fn link_token(text: &str, path: &str) -> String {
    assert!(!text.contains(&format!("{path}?token=")), "Token gehört ins Fragment: {text}");
    let start = text.find(&format!("{path}#token=")).unwrap_or_else(|| panic!("kein {path}-Link in: {text}"));
    text[start + path.len() + 7..].split_whitespace().next().unwrap().to_owned()
}

/// „Passwort vergessen“ – die Arbeit (Konto suchen, Mail) läuft nach der Antwort im Hintergrund; kurz
/// warten, damit Mails und Tokens danach sicher da sind (bzw. sicher ausbleiben).
async fn forgot(app: &TestApp, login: &str) -> (StatusCode, Value) {
    let res = app.call("POST", "/auth/password/forgot", None, Some(json!({ "login": login }))).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    res
}

/// 6-stelliger Code aus einer 2FA-Mail.
fn mail_code(text: &str) -> String {
    text.split_whitespace()
        .find(|w| w.len() == 6 && w.chars().all(|c| c.is_ascii_digit()))
        .unwrap()
        .to_owned()
}

async fn login(app: &TestApp, username: &str, password: &str) -> (StatusCode, Value) {
    app.call("POST", "/auth/login", None, Some(json!({ "username": username, "password": password }))).await
}

/// Hinterlegt und bestätigt eine Adresse.
async fn confirm_email(app: &TestApp, mailer: &CollectingMailer, token: &str, email: &str) {
    let (status, body) = app
        .call("PUT", "/auth/email", Some(token), Some(json!({ "email": email, "password": PASSWORD })))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let mails = mailer.take();
    assert_eq!(mails.len(), 1);
    let verify = link_token(&mails[0].text, "/verify-email");
    let (status, body) = app.call("POST", "/auth/email/verify", None, Some(json!({ "token": verify }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// TOTP-Geheimnis aus der Datenbank (Tests dürfen das; die API gibt es nur bei der Einrichtung heraus).
async fn totp_secret(app: &TestApp, username: &str, pending: bool) -> Vec<u8> {
    let column = if pending { "totp_pending_secret" } else { "totp_secret" };
    let sql =
        format!("SELECT m.{column} FROM user_mfa m JOIN users u ON u.id = m.user_id WHERE u.username = $1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(username).fetch_one(&app.db).await.unwrap()
}

/// Gültiger Code für einen noch nicht verbrauchten Zeitschritt (> `after`).
fn totp_code(secret: &[u8], after: Option<i64>) -> (String, i64) {
    let now = mfa::current_step();
    let step = (now - 1..=now + 1).find(|s| after.is_none_or(|a| *s > a)).expect("freier Zeitschritt");
    (format!("{:06}", mfa::totp_at(secret, step)), step)
}

/// Richtet TOTP ein; gibt Geheimnis, verbrauchten Zeitschritt und Wiederherstellungscodes zurück.
async fn enable_totp(app: &TestApp, token: &str, username: &str) -> (Vec<u8>, i64, Vec<String>) {
    let (status, setup) =
        app.call("POST", "/auth/mfa/totp/setup", Some(token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK, "{setup}");
    assert!(s(&setup["otpauth_uri"]).starts_with("otpauth://totp/"));
    assert!(s(&setup["qr_svg"]).contains("<svg"));
    let secret = totp_secret(app, username, true).await;
    assert_eq!(s(&setup["secret"]), mfa::base32(&secret));
    let (code, step) = totp_code(&secret, None);
    let (status, body) =
        app.call("POST", "/auth/mfa/totp/enable", Some(token), Some(json!({ "code": code }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let codes: Vec<String> =
        body["recovery_codes"].as_array().unwrap().iter().map(|c| s(c).to_owned()).collect();
    (secret, step, codes)
}

#[sqlx::test]
async fn instance_reports_email(db: PgPool) {
    let plain = TestApp::new(db.clone());
    let (_, info) = plain.call("GET", "/instance", None, None).await;
    assert_eq!(info["email"], false);
    assert_eq!(info["oidc_providers"], json!([]));
    // Ohne Mailversand gibt es weder Adresse noch Reset.
    let (status, body) =
        plain.call("POST", "/auth/password/forgot", None, Some(json!({ "login": "x" }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "email_unavailable"));
    let (status, body) = plain
        .call(
            "POST",
            "/auth/register",
            None,
            Some(json!({ "username": "alex", "password": PASSWORD, "email": "a@example.org" })),
        )
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "email_unavailable"));

    let (app, _) = app(db).await;
    let (_, info) = app.call("GET", "/instance", None, None).await;
    assert_eq!(info["email"], true);
}

#[sqlx::test]
async fn email_at_registration_needs_confirmation(db: PgPool) {
    let (app, mailer) = app(db).await;
    let (status, body) = app
        .call(
            "POST",
            "/auth/register",
            None,
            Some(json!({ "username": "alex", "password": PASSWORD, "email": "Alex@Example.org" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let token = s(&body["token"]).to_owned();
    // E-Mail gehört nicht ins öffentliche Profil.
    assert!(body["user"].get("email").is_none());

    let (_, sec) = app.call("GET", "/auth/security", Some(&token), None).await;
    assert_eq!(sec["email"], Value::Null);
    assert_eq!(sec["pending_email"], "Alex@Example.org");
    assert_eq!(sec["has_password"], true);

    // Unbestätigt taugt sie nicht für den Reset.
    let (status, _) = forgot(&app, "alex@example.org").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let mails = mailer.take();
    assert_eq!(mails.len(), 1, "nur die Bestätigungsmail");
    assert_eq!(mails[0].to, "Alex@Example.org");
    assert!(mails[0].subject.contains("bestätigen"));

    let verify = link_token(&mails[0].text, "/verify-email");
    let (status, _) = app.call("POST", "/auth/email/verify", None, Some(json!({ "token": "falsch" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = app.call("POST", "/auth/email/verify", None, Some(json!({ "token": verify }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["email"], "Alex@Example.org");
    // Einmalig.
    let (status, body) = app.call("POST", "/auth/email/verify", None, Some(json!({ "token": verify }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_token"));

    let (_, sec) = app.call("GET", "/auth/security", Some(&token), None).await;
    assert_eq!(sec["email"], "Alex@Example.org");
    assert_eq!(sec["pending_email"], Value::Null);

    // Ungültige Adresse.
    let (status, body) = app
        .call("PUT", "/auth/email", Some(&token), Some(json!({ "email": "kein-at", "password": PASSWORD })))
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_email"));
}

#[sqlx::test]
async fn email_change_requires_reauth_and_does_not_leak(db: PgPool) {
    let (app, mailer) = app(db).await;
    let alex = app.user("alex", None).await;
    let bob = app.user("bob", None).await;
    confirm_email(&app, &mailer, &alex, "alex@example.org").await;

    // Ohne bzw. mit falschem Passwort.
    let (status, body) =
        app.call("PUT", "/auth/email", Some(&bob), Some(json!({ "email": "bob@example.org" }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "wrong_password"));
    let (status, _) = app
        .call(
            "PUT",
            "/auth/email",
            Some(&bob),
            Some(json!({ "email": "bob@example.org", "password": "falsch" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Fremde, bestätigte Adresse: gleiche Antwort wie sonst, aber keine Mail.
    let (status, taken) = app
        .call(
            "PUT",
            "/auth/email",
            Some(&bob),
            Some(json!({ "email": "ALEX@example.org", "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(mailer.take().is_empty());
    let (status, free) = app
        .call(
            "PUT",
            "/auth/email",
            Some(&bob),
            Some(json!({ "email": "bob@example.org", "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        taken.as_object().unwrap().keys().collect::<Vec<_>>(),
        free.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    let bob_link = link_token(&mailer.take()[0].text, "/verify-email");

    // Alex wechselt auf eine Adresse, die Bob noch nicht bestätigt hat, und bestätigt zuerst.
    let (status, _) = app
        .call(
            "PUT",
            "/auth/email",
            Some(&alex),
            Some(json!({ "email": "Bob@Example.org", "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let mails = mailer.take();
    let alex_link = link_token(&mails[0].text, "/verify-email");
    let (status, _) = app.call("POST", "/auth/email/verify", None, Some(json!({ "token": alex_link }))).await;
    assert_eq!(status, StatusCode::OK);
    // Hinweis an die alte Adresse.
    let notice = mailer.take();
    assert_eq!(notice.len(), 1);
    assert_eq!(notice[0].to, "alex@example.org");
    assert!(notice[0].subject.contains("geändert"), "{}", notice[0].subject);
    // Die neue Adresse nur maskiert.
    assert!(!notice[0].text.contains("Bob@Example.org") && notice[0].text.contains("B•••@Example.org"));
    // Bobs Link scheitert jetzt an der Eindeutigkeit (ohne Groß/Klein).
    let (status, body) =
        app.call("POST", "/auth/email/verify", None, Some(json!({ "token": bob_link }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "email_taken"));

    // Entfernen.
    let (status, _) =
        app.call("DELETE", "/auth/email", Some(&alex), Some(json!({ "password": "falsch" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        app.call("DELETE", "/auth/email", Some(&alex), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, sec) = app.call("GET", "/auth/security", Some(&alex), None).await;
    assert_eq!(sec["email"], Value::Null);
}

#[sqlx::test]
async fn password_reset_flow(db: PgPool) {
    let (app, mailer) = app(db.clone()).await;
    let alex = app.user("alex", None).await;
    let _bob = app.user("bob", None).await;
    confirm_email(&app, &mailer, &alex, "alex@example.org").await;
    mailer.take();

    // Unbekannt, ohne Adresse, per Name, per Adresse: immer dieselbe Antwort.
    let mut answers = Vec::new();
    for login in ["niemand", "bob", "niemand@example.org", "ALEX", "Alex@Example.org"] {
        let (status, body) = forgot(&app, login).await;
        answers.push((status, body));
    }
    assert!(answers.iter().all(|a| a == &answers[0]), "{answers:?}");
    assert_eq!(answers[0].0, StatusCode::ACCEPTED);
    let mails = mailer.take();
    assert_eq!(mails.len(), 2, "nur für alex (per Name und per Adresse)");
    assert!(mails.iter().all(|m| m.to == "alex@example.org"));
    let first = link_token(&mails[0].text, "/reset-password");
    let second = link_token(&mails[1].text, "/reset-password");

    // Ein neuer Link ersetzt den älteren; Prüfen vorab wie bei Admin-Links (gemeinsame Tabelle).
    let verify = |token: &str| json!({ "token": token });
    let (status, body) = app.call("POST", "/auth/password-reset/verify", None, Some(verify(&first))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::NOT_FOUND, "invalid_reset_token"));
    let (status, info) = app.call("POST", "/auth/password-reset/verify", None, Some(verify(&second))).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["username"], "alex");
    let kind: String = sqlx::query_scalar("SELECT kind FROM password_reset_tokens WHERE used_at IS NULL")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(kind, "email_reset");
    // E-Mail-Links gelten 30 Minuten (Toleranz für Uhrabweichung zwischen Server und Datenbank).
    let minutes: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - now())::float8 / 60 FROM password_reset_tokens WHERE used_at IS NULL",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!((28.0..=31.0).contains(&minutes), "{minutes}");

    // Pro Konto höchstens 3 Mails pro Stunde – danach still keine weitere.
    let (status, _) = forgot(&app, "alex").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let third = link_token(&mailer.take()[0].text, "/reset-password");
    let (status, _) = forgot(&app, "alex").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(mailer.take().is_empty());

    let complete = |token: &str, password: &str| json!({ "token": token, "new_password": password });
    // Abgelaufener Link.
    sqlx::query("UPDATE password_reset_tokens SET expires_at = now() - interval '1 minute'")
        .execute(&db)
        .await
        .unwrap();
    let (status, body) =
        app.call("POST", "/auth/password-reset/complete", None, Some(complete(&third, NEW_PASSWORD))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::NOT_FOUND, "invalid_reset_token"));
    sqlx::query("UPDATE password_reset_tokens SET expires_at = now() + interval '30 minutes'")
        .execute(&db)
        .await
        .unwrap();

    // Zu kurzes Passwort.
    let (status, _) =
        app.call("POST", "/auth/password-reset/complete", None, Some(complete(&third, "kurz"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Halbe 2FA-Anmeldungen gibt es danach nicht mehr.
    sqlx::query(
        "INSERT INTO mfa_challenges (token_hash, user_id, expires_at)
         SELECT '\\x00'::bytea, id, now() + interval '5 minutes' FROM users WHERE username = 'alex'",
    )
    .execute(&db)
    .await
    .unwrap();
    let (status, _) =
        app.call("POST", "/auth/password-reset/complete", None, Some(complete(&third, NEW_PASSWORD))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let challenges: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mfa_challenges").fetch_one(&db).await.unwrap();
    assert_eq!(challenges, 0);
    // Alle Sitzungen sind beendet, das alte Passwort gilt nicht mehr.
    let (status, _) = app.call("GET", "/auth/me", Some(&alex), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "alex", PASSWORD).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&app, "alex", NEW_PASSWORD).await.0, StatusCode::OK);
    // Einmalig.
    let (status, _) =
        app.call("POST", "/auth/password-reset/complete", None, Some(complete(&third, PASSWORD))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn disabled_accounts_are_rejected(db: PgPool) {
    let (app, mailer) = app(db.clone()).await;
    let admin = app.user("admin", None).await;
    let bob = app.user("bob", None).await;
    let bob_id = app.user_id(&bob).await;
    confirm_email(&app, &mailer, &bob, "bob@example.org").await;
    let (status, _) =
        app.call("POST", "/auth/mfa/email", Some(&bob), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK);

    // Laufende 2FA-Anmeldung und offener Reset-Link von vor der Sperre.
    let (_, body) = login(&app, "bob", PASSWORD).await;
    let mfa_token = s(&body["mfa_token"]).to_owned();
    forgot(&app, "bob").await;
    let reset = link_token(&mailer.take()[0].text, "/reset-password");

    let (status, _) = app.call("POST", &format!("/admin/users/{bob_id}/disable"), Some(&admin), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let forbidden = |(status, body): (StatusCode, Value)| {
        assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "account_disabled"), "{body}");
    };
    // Passwort richtig, aber gesperrt: keine 2FA-Challenge.
    forbidden(login(&app, "bob", PASSWORD).await);
    // Zweiter Schritt einer vorher begonnenen Anmeldung, auch per Wiederherstellungscode.
    forbidden(
        app.call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": mfa_token, "method": "recovery", "code": "x" })),
        )
        .await,
    );
    // Kein E-Mail-Code.
    forbidden(app.call("POST", "/auth/login/mfa/email", None, Some(json!({ "mfa_token": mfa_token }))).await);
    assert!(mailer.take().is_empty());
    // Reset-Link: weder prüfen noch einlösen.
    forbidden(app.call("POST", "/auth/password-reset/verify", None, Some(json!({ "token": reset }))).await);
    forbidden(
        app.call(
            "POST",
            "/auth/password-reset/complete",
            None,
            Some(json!({ "token": reset, "new_password": NEW_PASSWORD })),
        )
        .await,
    );
    // „Passwort vergessen“: gleiche Antwort, aber keine Mail.
    let (status, _) = forgot(&app, "bob").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(mailer.take().is_empty());
}

#[sqlx::test]
async fn admin_deletion_removes_security_data(db: PgPool) {
    let (app, mailer) = app(db.clone()).await;
    let admin = app.user("admin", None).await;
    let bob = app.user("bob", None).await;
    let bob_id = app.user_id(&bob).await;
    confirm_email(&app, &mailer, &bob, "bob@example.org").await;
    enable_totp(&app, &bob, "bob").await;
    forgot(&app, "bob").await;
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id) VALUES ('mock', 'bob-sub', $1::uuid)",
    )
    .bind(&bob_id)
    .execute(&db)
    .await
    .unwrap();

    let (status, body) =
        app.call("DELETE", &format!("/admin/users/{bob_id}"), Some(&admin), Some(json!({}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for table in
        ["user_mfa", "mfa_recovery_codes", "user_identities", "password_reset_tokens", "email_verifications"]
    {
        let sql = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).fetch_one(&db).await.unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let emails: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email IS NOT NULL")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(emails, 0);
}

#[sqlx::test]
async fn password_forgot_rate_limit_per_ip(db: PgPool) {
    let config = Config { trust_proxy: true, ..Config::for_tests() };
    let (app, _) = TestApp::with_mailer(db, config);
    let forgot = |ip: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/auth/password/forgot")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-forwarded-for", ip)
            .body(Body::from(json!({ "login": "irgendwer" }).to_string()))
            .unwrap()
    };
    for _ in 0..5 {
        assert_eq!(app.send(forgot("203.0.113.9")).await.0, StatusCode::ACCEPTED);
    }
    assert_eq!(app.send(forgot("203.0.113.9")).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(app.send(forgot("203.0.113.10")).await.0, StatusCode::ACCEPTED);
}

#[sqlx::test]
async fn totp_login_flow(db: PgPool) {
    let (app, _) = app(db.clone()).await;
    let token = app.user("alex", None).await;

    // Einrichtung braucht das Passwort; falscher erster Code aktiviert nichts.
    let (status, _) =
        app.call("POST", "/auth/mfa/totp/setup", Some(&token), Some(json!({ "password": "falsch" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        app.call("POST", "/auth/mfa/totp/enable", Some(&token), Some(json!({ "code": "123456" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ohne Setup");
    let (status, _) =
        app.call("POST", "/auth/mfa/totp/setup", Some(&token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK);
    let secret = totp_secret(&app, "alex", true).await;
    let (good, _) = totp_code(&secret, None);
    let wrong = format!("{:06}", (good.parse::<u32>().unwrap() + 1) % 1_000_000);
    let (status, body) =
        app.call("POST", "/auth/mfa/totp/enable", Some(&token), Some(json!({ "code": wrong }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "invalid_code"));

    let (secret, used_step, recovery) = enable_totp(&app, &token, "alex").await;
    assert_eq!(recovery.len(), 10);
    let (_, sec) = app.call("GET", "/auth/security", Some(&token), None).await;
    assert_eq!(sec["mfa"], json!({ "totp": true, "email": false, "recovery_codes_left": 10 }));
    // Das Geheimnis ist nie wieder abrufbar.
    assert!(!sec.to_string().contains(&mfa::base32(&secret)));

    // Passwort allein reicht nicht mehr.
    let (status, body) = login(&app, "alex", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mfa_required"], true);
    assert!(body.get("token").is_none());
    assert_eq!(body["methods"], json!(["totp", "recovery"]));
    let mfa_token = s(&body["mfa_token"]).to_owned();
    // Das mfa_token ist keine Session.
    assert_eq!(app.call("GET", "/auth/me", Some(&mfa_token), None).await.0, StatusCode::UNAUTHORIZED);

    let step2 = |method: &str, code: &str| json!({ "mfa_token": mfa_token, "method": method, "code": code });
    // Falscher Code; Replay des Codes, mit dem TOTP aktiviert wurde; nicht eingerichtete Methode.
    let (status, body) = app.call("POST", "/auth/login/mfa", None, Some(step2("totp", &wrong))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::UNAUTHORIZED, "invalid_code"));
    let replay = format!("{:06}", mfa::totp_at(&secret, used_step));
    let (status, _) = app.call("POST", "/auth/login/mfa", None, Some(step2("totp", &replay))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "Replay");
    let (status, _) = app.call("POST", "/auth/login/mfa", None, Some(step2("email", "000000"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (code, _) = totp_code(&secret, Some(used_step));
    let (status, body) = app.call("POST", "/auth/login/mfa", None, Some(step2("totp", &code))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = s(&body["token"]).to_owned();
    assert_eq!(app.call("GET", "/auth/me", Some(&session), None).await.0, StatusCode::OK);
    // Challenge ist verbraucht.
    let (status, body) = app.call("POST", "/auth/login/mfa", None, Some(step2("totp", &code))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::UNAUTHORIZED, "invalid_mfa_token"));

    // Derselbe Code gilt auch bei einer neuen Anmeldung nicht noch einmal.
    let (_, body) = login(&app, "alex", PASSWORD).await;
    let t2 = s(&body["mfa_token"]).to_owned();
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t2, "method": "totp", "code": code })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "Replay über Sitzungen hinweg");

    // Wiederherstellungscode: einmal gültig, Schreibweise egal.
    let rc = recovery[0].to_uppercase().replace('-', " ");
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t2, "method": "recovery", "code": rc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = login(&app, "alex", PASSWORD).await;
    let t3 = s(&body["mfa_token"]).to_owned();
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t3, "method": "recovery", "code": recovery[0] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, sec) = app.call("GET", "/auth/security", Some(&session), None).await;
    assert_eq!(sec["mfa"]["recovery_codes_left"], 9);

    // Nach 5 Fehlversuchen ist die Challenge verbraucht – auch für den richtigen Code.
    for _ in 0..4 {
        app.call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t3, "method": "totp", "code": wrong })),
        )
        .await;
    }
    let (status, body) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t3, "method": "recovery", "code": recovery[1] })),
        )
        .await;
    assert_eq!((status, s(&body["error"])), (StatusCode::UNAUTHORIZED, "invalid_mfa_token"));

    // Abgelaufene Challenge.
    let (_, body) = login(&app, "alex", PASSWORD).await;
    let t4 = s(&body["mfa_token"]).to_owned();
    sqlx::query("UPDATE mfa_challenges SET expires_at = now() - interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t4, "method": "recovery", "code": recovery[1] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn mfa_failures_hit_login_rate_limit(db: PgPool) {
    let (app, _) = app(db).await;
    let token = app.user("alex", None).await;
    enable_totp(&app, &token, "alex").await;
    // 10 Fehlversuche (über mehrere Challenges) sperren den Benutzernamen.
    for _ in 0..2 {
        let (_, body) = login(&app, "alex", PASSWORD).await;
        let t = s(&body["mfa_token"]).to_owned();
        for _ in 0..5 {
            app.call(
                "POST",
                "/auth/login/mfa",
                None,
                Some(json!({ "mfa_token": t, "method": "recovery", "code": "x" })),
            )
            .await;
        }
    }
    let (status, _) = login(&app, "alex", PASSWORD).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn sensitive_actions_require_second_factor(db: PgPool) {
    let (app, _) = app(db).await;
    let token = app.user("alex", None).await;
    let (secret, step, recovery) = enable_totp(&app, &token, "alex").await;

    // Passwort ändern: ohne Code, mit falschem Code, mit richtigem.
    let change = |code: Option<&str>| json!({ "current_password": PASSWORD, "new_password": NEW_PASSWORD, "mfa_code": code });
    let (status, body) = app.call("POST", "/auth/password", Some(&token), Some(change(None))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "mfa_code_required"));
    let (status, body) = app.call("POST", "/auth/password", Some(&token), Some(change(Some("000000")))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "invalid_code"));
    let (code, step) = totp_code(&secret, Some(step));
    let (status, body) = app.call("POST", "/auth/password", Some(&token), Some(change(Some(&code)))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // TOTP abschalten: Wiederherstellungscode als zweiter Faktor.
    let (status, _) =
        app.call("DELETE", "/auth/mfa/totp", Some(&token), Some(json!({ "password": NEW_PASSWORD }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = app
        .call(
            "DELETE",
            "/auth/mfa/totp",
            Some(&token),
            Some(json!({ "password": NEW_PASSWORD, "mfa_method": "recovery", "mfa_code": recovery[2] })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, sec) = app.call("GET", "/auth/security", Some(&token), None).await;
    assert_eq!(sec["mfa"], json!({ "totp": false, "email": false, "recovery_codes_left": 0 }));
    // Ohne 2FA ist die Anmeldung wieder einstufig.
    let (_, body) = login(&app, "alex", NEW_PASSWORD).await;
    assert!(body["token"].is_string());
    let _ = step;
}

#[sqlx::test]
async fn email_second_factor(db: PgPool) {
    let (app, mailer) = app(db.clone()).await;
    let token = app.user("alex", None).await;

    // Ohne bestätigte Adresse nicht möglich.
    let (status, body) =
        app.call("POST", "/auth/mfa/email", Some(&token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::BAD_REQUEST, "email_unavailable"));
    confirm_email(&app, &mailer, &token, "alex@example.org").await;
    let (status, body) =
        app.call("POST", "/auth/mfa/email", Some(&token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recovery_codes"].as_array().unwrap().len(), 10);

    // Adresse entfernen geht erst, wenn 2FA per E-Mail aus ist.
    let (status, body) =
        app.call("DELETE", "/auth/email", Some(&token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "email_mfa_active"));

    let (_, body) = login(&app, "alex", PASSWORD).await;
    assert_eq!(body["methods"], json!(["email", "recovery"]));
    let t = s(&body["mfa_token"]).to_owned();
    // TOTP ist nicht eingerichtet → zählt nicht.
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t, "method": "totp", "code": "123456" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = app.call("POST", "/auth/login/mfa/email", None, Some(json!({ "mfa_token": t }))).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let mails = mailer.take();
    assert_eq!(mails.len(), 1);
    let code = mail_code(&mails[0].text);
    let wrong = format!("{:06}", (code.parse::<u32>().unwrap() + 1) % 1_000_000);
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t, "method": "email", "code": wrong })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t, "method": "email", "code": code })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Code ist verbraucht.
    let (_, body) = login(&app, "alex", PASSWORD).await;
    let t2 = s(&body["mfa_token"]).to_owned();
    let (status, _) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": t2, "method": "email", "code": code })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Code zur Bestätigung einer sensiblen Aktion; nach 5 Fehlversuchen hilft auch der richtige nicht mehr.
    let (status, _) = app.call("POST", "/auth/mfa/email-code", Some(&token), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let code = mail_code(&mailer.take()[0].text);
    let wrong = format!("{:06}", (code.parse::<u32>().unwrap() + 1) % 1_000_000);
    let regenerate = |code: &str| json!({ "password": PASSWORD, "mfa_method": "email", "mfa_code": code });
    for _ in 0..5 {
        let (status, body) =
            app.call("POST", "/auth/mfa/recovery-codes", Some(&token), Some(regenerate(&wrong))).await;
        assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "invalid_code"));
    }
    let (status, _) =
        app.call("POST", "/auth/mfa/recovery-codes", Some(&token), Some(regenerate(&code))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Versuchslimit erreicht");

    // Höchstens 5 Mails pro Konto und Stunde (bisher: Bestätigung + 2 Codes).
    for _ in 0..2 {
        assert_eq!(
            app.call("POST", "/auth/mfa/email-code", Some(&token), None).await.0,
            StatusCode::ACCEPTED
        );
    }
    assert_eq!(
        app.call("POST", "/auth/mfa/email-code", Some(&token), None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test]
async fn export_and_delete_with_mfa(db: PgPool) {
    let (app, mailer) = app(db.clone()).await;
    // Der erste Nutzer wird Instanz-Admin und dürfte sich nicht löschen.
    app.user("admin", None).await;
    let token = app.user("alex", None).await;
    confirm_email(&app, &mailer, &token, "alex@example.org").await;
    let (secret, step, _) = enable_totp(&app, &token, "alex").await;

    let req = Request::builder()
        .uri("/api/auth/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let (status, export) = app.send(req).await;
    assert_eq!(status, StatusCode::OK);
    let sec = &export["account_security"];
    assert_eq!(sec["email"], "alex@example.org");
    assert_eq!(sec["two_factor"]["totp"], true);
    assert_eq!(sec["identities"], json!([]));
    let text = export.to_string();
    assert!(!text.contains(&mfa::base32(&secret)));
    assert!(!text.contains("secret"));
    assert!(!text.contains("code_hash"));

    // Konto löschen braucht Passwort und Code.
    let (status, body) =
        app.call("DELETE", "/auth/me", Some(&token), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "mfa_code_required"));
    let (code, _) = totp_code(&secret, Some(step));
    let (status, body) = app
        .call("DELETE", "/auth/me", Some(&token), Some(json!({ "password": PASSWORD, "mfa_code": code })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for table in ["user_mfa", "mfa_recovery_codes", "email_verifications"] {
        let sql = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).fetch_one(&db).await.unwrap();
        assert_eq!(n, 0, "{table}");
    }
}
