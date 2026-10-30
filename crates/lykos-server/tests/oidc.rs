//! Paket G: Anmeldung über OpenID Connect gegen einen lokalen Mock-Anbieter mit eigenen Schlüsseln.

mod common;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    routing::{get, post},
};
use common::{PASSWORD, TestApp, s};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use lykos_server::{
    config::{AuthProtocol, Config, OAuth2Endpoints, OidcProviderConfig},
    mfa, oidc,
};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Wegwerf-Schlüssel nur für Tests: Ed25519, PKCS#8-DER als Base64.
const KEY: &str = "MC4CAQAwBQYDK2VwBCIEINnINSmJysz0U5BBYOOvuBkqbjnrfQCPtXij/dWy1X4u";
const KEY_X: &str = "XlhEyP776J7FlZQ1PGee51C3oP0DhBNN69x0E8et07Y";
const CLIENT_ID: &str = "lykos";
const CLIENT_SECRET: &str = "geheim";

/// Negativfall: Name und Abweichung.
type Case = (&'static str, fn(&mut Next));

/// Was der Mock-Anbieter beim nächsten Token-Abruf ausstellt.
#[derive(Default)]
struct Next {
    /// Erwartete PKCE-Challenge (aus der Autorisierungs-URL).
    challenge: String,
    nonce: String,
    subject: String,
    email: Option<String>,
    /// Abweichungen für Negativtests.
    nonce_override: Option<String>,
    aud_override: Option<String>,
    iss_override: Option<String>,
    expired: bool,
}

#[derive(Clone)]
struct Mock {
    base: String,
    next: Arc<Mutex<Next>>,
}

async fn discovery(State(m): State<Mock>) -> Json<Value> {
    Json(json!({
        "issuer": m.base,
        "authorization_endpoint": format!("{}/authorize", m.base),
        "token_endpoint": format!("{}/token", m.base),
        "jwks_uri": format!("{}/jwks", m.base),
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
    }))
}

async fn jwks() -> Json<Value> {
    Json(json!({ "keys": [{ "kty": "OKP", "crv": "Ed25519", "x": KEY_X, "kid": "mock-1", "alg": "EdDSA" }] }))
}

async fn token(State(m): State<Mock>, headers: HeaderMap, body: String) -> Result<Json<Value>, StatusCode> {
    let param = |name: &str| {
        body.split('&')
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .map(String::from)
            .unwrap_or_default()
    };
    // Client-Authentifizierung: client_secret_basic (OIDC) oder client_secret_post (OAuth2-Presets).
    let expected = format!(
        "Basic {}",
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{CLIENT_ID}:{CLIENT_SECRET}")
        )
    );
    let basic = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some(expected.as_str());
    let post = param("client_id") == CLIENT_ID && param("client_secret") == CLIENT_SECRET;
    if !basic && !post {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let next = m.next.lock().unwrap();
    // PKCE: Der Verifier muss zur Challenge aus der Autorisierungs-URL passen.
    if param("grant_type") != "authorization_code"
        || oidc::pkce_challenge(&param("code_verifier")) != next.challenge
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let now = jsonwebtoken::get_current_timestamp();
    let mut claims = json!({
        "iss": next.iss_override.clone().unwrap_or_else(|| m.base.clone()),
        "aud": next.aud_override.clone().unwrap_or_else(|| CLIENT_ID.into()),
        "sub": next.subject,
        "iat": now,
        "exp": if next.expired { now - 600 } else { now + 300 },
        "nonce": next.nonce_override.clone().unwrap_or_else(|| next.nonce.clone()),
        // Profildaten, die Lykos ignorieren muss.
        "name": "Max Mustermann",
        "picture": "https://idp.example/bild.png",
    });
    if let Some(email) = &next.email {
        claims["email"] = json!(email);
        claims["email_verified"] = json!(true);
    }
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("mock-1".into());
    let id_token = jsonwebtoken::encode(
        &header,
        &claims,
        &EncodingKey::from_ed_der(
            &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, KEY).unwrap(),
        ),
    )
    .unwrap();
    let access_token = format!("at-{}", next.subject);
    Ok(Json(json!({ "access_token": access_token, "token_type": "Bearer", "id_token": id_token })))
}

/// Userinfo im Stil von Discord (`id`, `email`, `verified`) für den reinen OAuth2-Weg.
async fn userinfo(State(m): State<Mock>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let next = m.next.lock().unwrap();
    let expected = format!("Bearer at-{}", next.subject);
    if headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some(expected.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(json!({
        "id": next.subject,
        "username": "wolf_im_netz",
        "avatar": "a_123",
        "email": next.email,
        "verified": true,
    })))
}

async fn mock_provider() -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mock = Mock { base, next: Arc::default() };
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .with_state(mock.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    mock
}

/// Zwei Anbieter auf demselben Mock: `mock` (OIDC mit ID-Token) und `plain` (reines OAuth2 mit Userinfo,
/// wie Discord).
async fn setup(db: PgPool) -> (TestApp, Mock) {
    let mock = mock_provider().await;
    let provider = |id: &str, label: &str, scopes: &[&str], protocol: AuthProtocol| OidcProviderConfig {
        id: id.into(),
        label: label.into(),
        client_id: CLIENT_ID.into(),
        client_secret: Some(CLIENT_SECRET.into()),
        scopes: scopes.iter().map(|s| (*s).into()).collect(),
        protocol,
    };
    let config = Config {
        oidc_providers: vec![
            provider(
                "mock",
                "Mock-IdP",
                &["openid", "email"],
                AuthProtocol::Oidc { issuer: mock.base.clone() },
            ),
            provider(
                "plain",
                "Plain-OAuth",
                &["identify", "email"],
                AuthProtocol::OAuth2(OAuth2Endpoints {
                    authorize_url: format!("{}/authorize", mock.base),
                    token_url: format!("{}/token", mock.base),
                    userinfo_url: format!("{}/userinfo", mock.base),
                    subject_field: "id".into(),
                    email_field: Some("email".into()),
                    email_verified_field: Some("verified".into()),
                }),
            ),
        ],
        ..Config::for_tests()
    };
    (TestApp::with_config(db, config), mock)
}

/// Query-Parameter einer URL (die hier relevanten Werte sind URL-sicher).
fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?').unwrap().1;
    query.split('&').find_map(|p| p.strip_prefix(&format!("{name}="))).map(String::from)
}

/// Startet einen Ablauf beim Anbieter `mock` und richtet den Mock auf `subject` aus. Gibt den `state` zurück.
async fn start(app: &TestApp, mock: &Mock, mode: &str, token: Option<&str>, subject: &str) -> String {
    start_with(app, mock, "mock", mode, token, subject).await
}

async fn start_with(
    app: &TestApp,
    mock: &Mock,
    provider: &str,
    mode: &str,
    token: Option<&str>,
    subject: &str,
) -> String {
    let path = format!("/auth/oidc/{provider}/start");
    // Das Passwort braucht nur `link` (Re-Authentifizierung); bei `login` wird es ignoriert.
    let body = json!({ "mode": mode, "session_label": "Web", "password": PASSWORD });
    let (status, body) = app.call("POST", &path, token, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = s(&body["url"]);
    assert!(url.starts_with(&format!("{}/authorize?", mock.base)));
    let param = |name: &str| query_param(url, name);
    assert_eq!(param("client_id").unwrap(), CLIENT_ID);
    assert_eq!(param("code_challenge_method").unwrap(), "S256");
    assert!(
        param("redirect_uri").unwrap().ends_with(&format!("%2Fapi%2Fauth%2Foidc%2F{provider}%2Fcallback"))
    );
    if provider == "mock" {
        assert_eq!(param("scope").unwrap(), "openid%20email");
    } else {
        // Reines OAuth2: kein nonce.
        assert_eq!(param("scope").unwrap(), "identify%20email");
        assert_eq!(param("nonce"), None);
    }
    let mut next = mock.next.lock().unwrap();
    *next = Next {
        challenge: param("code_challenge").unwrap(),
        nonce: param("nonce").unwrap_or_default(),
        subject: subject.into(),
        email: Some(format!("{subject}@idp.example.org")),
        ..Next::default()
    };
    param("state").unwrap()
}

/// Ruft den Callback auf; gibt Status, `Location` und Header zurück.
async fn callback(app: &TestApp, state: &str, cookie: Option<&str>) -> (StatusCode, String, HeaderMap) {
    callback_with(app, "mock", state, cookie).await
}

async fn callback_with(
    app: &TestApp,
    provider: &str,
    state: &str,
    cookie: Option<&str>,
) -> (StatusCode, String, HeaderMap) {
    let mut req =
        Request::builder().uri(format!("/api/auth/oidc/{provider}/callback?code=abc&state={state}"));
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, format!("lykos_oidc={c}"));
    }
    let (status, headers, _) = app.send_full(req.body(Body::empty()).unwrap()).await;
    let location = headers.get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
    (status, location, headers)
}

/// Kompletter Ablauf mit passendem Cookie.
async fn flow(
    app: &TestApp,
    mock: &Mock,
    mode: &str,
    token: Option<&str>,
    subject: &str,
) -> (String, HeaderMap) {
    let state = start(app, mock, mode, token, subject).await;
    let (status, location, headers) = callback(app, &state, Some(&state)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    (location, headers)
}

fn fragment<'a>(location: &'a str, key: &str) -> &'a str {
    let frag = location.strip_prefix("/#").unwrap_or_else(|| panic!("kein Fragment: {location}"));
    frag.split('&')
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("{key} fehlt: {location}"))
}

fn session_from_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| c.strip_prefix("lykos_session="))
        .map(|c| c.split(';').next().unwrap().to_owned())
        .filter(|t| !t.is_empty())
}

async fn invite(app: &TestApp, owner: &str) -> String {
    let community = app.community(owner, "Rudel").await;
    let (_, invite) = app
        .call("POST", &format!("/communities/{}/invites", community.id), Some(owner), Some(json!({})))
        .await;
    s(&invite["code"]).to_owned()
}

#[sqlx::test]
async fn instance_lists_providers(db: PgPool) {
    let (app, _) = setup(db).await;
    let (_, info) = app.call("GET", "/instance", None, None).await;
    assert_eq!(
        info["oidc_providers"],
        json!([{ "id": "mock", "label": "Mock-IdP" }, { "id": "plain", "label": "Plain-OAuth" }])
    );
    let (status, _) =
        app.call("POST", "/auth/oidc/andere/start", None, Some(json!({ "mode": "login" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Verknüpfen nur angemeldet.
    let (status, _) = app.call("POST", "/auth/oidc/mock/start", None, Some(json!({ "mode": "link" }))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn signup_login_and_unlink(db: PgPool) {
    let (app, mock) = setup(db.clone()).await;
    let admin = app.user("admin", None).await;
    let code = invite(&app, &admin).await;

    let (location, headers) = flow(&app, &mock, "login", None, "sub-1").await;
    let signup = fragment(&location, "oidc_signup").to_owned();
    // Ablauf-Cookie wird gelöscht, aber noch keine Session.
    assert!(session_from_cookie(&headers).is_none());

    let (status, info) =
        app.call("POST", "/auth/oidc/signup", None, Some(json!({ "signup_token": signup }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        info,
        json!({ "provider": "mock", "provider_label": "Mock-IdP", "email": "sub-1@idp.example.org" })
    );

    // Gleiche Regeln wie bei der normalen Registrierung: Einladung nötig, Benutzername geprüft.
    let register = |username: &str, invite: Option<&str>| json!({ "signup_token": signup, "username": username, "invite": invite, "use_email": true });
    let (status, body) = app.call("POST", "/auth/oidc/register", None, Some(register("wolf", None))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "invite_required"));
    let (status, _) = app.call("POST", "/auth/oidc/register", None, Some(register("x", Some(&code)))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) =
        app.call("POST", "/auth/oidc/register", None, Some(register("admin", Some(&code)))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "username_taken"));
    let (status, body) =
        app.call("POST", "/auth/oidc/register", None, Some(register("wolf", Some(&code)))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let token = s(&body["token"]).to_owned();
    // Keine Profildaten vom Anbieter.
    assert_eq!(body["user"]["display_name"], Value::Null);
    assert_eq!(body["user"]["avatar_url"], Value::Null);
    // Token ist verbraucht.
    let (status, _) =
        app.call("POST", "/auth/oidc/register", None, Some(register("wolf2", Some(&code)))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, sec) = app.call("GET", "/auth/security", Some(&token), None).await;
    assert_eq!(sec["has_password"], false);
    assert_eq!(sec["email"], "sub-1@idp.example.org", "bestätigte Adresse nach Zustimmung übernommen");
    assert_eq!(sec["identities"][0]["provider"], "mock");
    assert_eq!(sec["identities"][0]["label"], "Mock-IdP");
    // Mitglied der Community aus der Einladung.
    let (_, communities) = app.call("GET", "/communities", Some(&token), None).await;
    assert_eq!(communities.as_array().unwrap().len(), 1);
    // Gleiche Beitrittslogik wie bei der Passwort-Registrierung, inkl. Systemnachricht (falls Systemkanal gesetzt).
    let system_channel: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT system_channel_id FROM communities").fetch_one(&db).await.unwrap();
    let join_messages: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM messages m JOIN users u ON u.id = m.author_id
         WHERE m.kind = 'member_join' AND u.username = 'wolf'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(join_messages, i64::from(system_channel.is_some()));

    // Passwort-Login gibt es für dieses Konto nicht.
    let (status, _) = app
        .call("POST", "/auth/login", None, Some(json!({ "username": "wolf", "password": PASSWORD })))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Erneute Anmeldung mit derselben Identität → Session-Cookie, zurück auf „/“.
    let (location, headers) = flow(&app, &mock, "login", None, "sub-1").await;
    assert_eq!(location, "/");
    let session = session_from_cookie(&headers).expect("Session-Cookie");
    let (_, me) = app.call("GET", "/auth/me", Some(&session), None).await;
    assert_eq!(me["username"], "wolf");

    // Einziger Anmeldeweg → Trennen verboten.
    let (status, body) = app.call("DELETE", "/auth/identities/mock", Some(&token), Some(json!({}))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::CONFLICT, "last_login_method"));

    // Ohne Passwort bestätigt eine frische Sitzung; eine alte reicht nicht.
    sqlx::query("UPDATE sessions SET created_at = now() - interval '1 hour'").execute(&db).await.unwrap();
    let (status, body) =
        app.call("POST", "/auth/password", Some(&token), Some(json!({ "new_password": PASSWORD }))).await;
    assert_eq!((status, s(&body["error"])), (StatusCode::FORBIDDEN, "reauth_required"));
    let (location, headers) = flow(&app, &mock, "login", None, "sub-1").await;
    assert_eq!(location, "/");
    let fresh = session_from_cookie(&headers).unwrap();
    let (status, body) = app
        .call(
            "POST",
            "/auth/password",
            Some(&fresh),
            Some(json!({ "new_password": PASSWORD, "logout_other_sessions": false })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // Jetzt mit Passwort: Trennen erlaubt (Passwort nötig).
    let (status, _) = app.call("DELETE", "/auth/identities/mock", Some(&fresh), Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .call("DELETE", "/auth/identities/mock", Some(&fresh), Some(json!({ "password": PASSWORD })))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .call("DELETE", "/auth/identities/mock", Some(&fresh), Some(json!({ "password": PASSWORD })))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call("POST", "/auth/login", None, Some(json!({ "username": "wolf", "password": PASSWORD })))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn email_is_only_taken_with_consent(db: PgPool) {
    let (app, mock) = setup(db).await;
    let (location, _) = flow(&app, &mock, "login", None, "sub-2").await;
    let signup = fragment(&location, "oidc_signup").to_owned();
    // Erster Nutzer (Bootstrap) braucht keine Einladung.
    let (status, body) = app
        .call(
            "POST",
            "/auth/oidc/register",
            None,
            Some(json!({ "signup_token": signup, "username": "erster" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["user"]["is_instance_admin"], true);
    let (_, sec) = app.call("GET", "/auth/security", Some(s(&body["token"])), None).await;
    assert_eq!(sec["email"], Value::Null);
}

#[sqlx::test]
async fn rejects_invalid_responses(db: PgPool) {
    let (app, mock) = setup(db).await;
    app.user("admin", None).await;

    // Falscher nonce, falsche Audience, falscher Issuer, abgelaufen.
    let cases: [Case; 4] = [
        ("nonce", |n| n.nonce_override = Some("anderer-nonce".into())),
        ("aud", |n| n.aud_override = Some("andere-app".into())),
        ("iss", |n| n.iss_override = Some("https://evil.example.org".into())),
        ("exp", |n| n.expired = true),
    ];
    for (name, tweak) in cases {
        let state = start(&app, &mock, "login", None, "sub-x").await;
        tweak(&mut mock.next.lock().unwrap());
        let (_, location, headers) = callback(&app, &state, Some(&state)).await;
        assert_eq!(fragment(&location, "oidc_error"), "oidc_failed", "{name}");
        assert!(session_from_cookie(&headers).is_none(), "{name}");
    }

    // PKCE: Mock lehnt ab, wenn der Verifier nicht zur Challenge passt.
    let state = start(&app, &mock, "login", None, "sub-x").await;
    mock.next.lock().unwrap().challenge = "falsch".into();
    let (_, location, _) = callback(&app, &state, Some(&state)).await;
    assert_eq!(fragment(&location, "oidc_error"), "oidc_failed");

    // Ohne bzw. mit fremdem Cookie (Login-CSRF) und bei Wiederverwendung des state.
    let state = start(&app, &mock, "login", None, "sub-x").await;
    let (_, location, _) = callback(&app, &state, None).await;
    assert_eq!(fragment(&location, "oidc_error"), "invalid_state");
    let (_, location, _) = callback(&app, &state, Some("anderer-state")).await;
    assert_eq!(fragment(&location, "oidc_error"), "invalid_state");
    let (_, location, _) = callback(&app, &state, Some(&state)).await;
    assert!(fragment(&location, "oidc_signup").len() > 20);
    let (_, location, _) = callback(&app, &state, Some(&state)).await;
    assert_eq!(fragment(&location, "oidc_error"), "invalid_state", "state nur einmal");

    // Abbruch beim Anbieter.
    let req = Request::builder()
        .uri("/api/auth/oidc/mock/callback?error=access_denied&state=x")
        .body(Body::empty())
        .unwrap();
    let (_, headers, _) = app.send_full(req).await;
    assert_eq!(headers[header::LOCATION], "/#oidc_error=cancelled");
}

#[sqlx::test]
async fn link_and_login_with_second_factor(db: PgPool) {
    let (app, mock) = setup(db.clone()).await;
    sqlx::query("UPDATE instance_settings SET value = '\"open\"' WHERE key = 'registration_mode'")
        .execute(&db)
        .await
        .unwrap();
    let alex = app.user("alex", None).await;
    let bob = app.user("bob", None).await;

    // Verknüpfen verlangt wie das Trennen das aktuelle Passwort.
    for (body, error) in [
        (json!({ "mode": "link" }), "wrong_password"),
        (json!({ "mode": "link", "password": "falsch" }), "wrong_password"),
    ] {
        let (status, res) = app.call("POST", "/auth/oidc/mock/start", Some(&alex), Some(body)).await;
        assert_eq!((status, res["error"].as_str()), (StatusCode::FORBIDDEN, Some(error)), "{res}");
    }
    let flows: i64 = sqlx::query_scalar("SELECT count(*) FROM oidc_flows").fetch_one(&db).await.unwrap();
    assert_eq!(flows, 0, "ohne Bestätigung kein Ablauf");

    let (location, _) = flow(&app, &mock, "link", Some(&alex), "alex-sub").await;
    assert_eq!(fragment(&location, "oidc_linked"), "mock");
    // Dieselbe Identität kann kein zweites Konto verknüpfen; ein Konto nur eine Identität pro Anbieter.
    let (location, _) = flow(&app, &mock, "link", Some(&bob), "alex-sub").await;
    assert_eq!(fragment(&location, "oidc_error"), "identity_in_use");
    let (location, _) = flow(&app, &mock, "link", Some(&alex), "zweite-sub").await;
    assert_eq!(fragment(&location, "oidc_error"), "already_linked");

    // Mit aktiver 2FA führt auch die OIDC-Anmeldung zum zweiten Schritt.
    let (status, _) =
        app.call("POST", "/auth/mfa/totp/setup", Some(&alex), Some(json!({ "password": PASSWORD }))).await;
    assert_eq!(status, StatusCode::OK);
    let secret: Vec<u8> = sqlx::query_scalar(
        "SELECT m.totp_pending_secret FROM user_mfa m JOIN users u ON u.id = m.user_id WHERE u.username = 'alex'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let now = mfa::current_step();
    let enable = format!("{:06}", mfa::totp_at(&secret, now - 1));
    let (status, _) =
        app.call("POST", "/auth/mfa/totp/enable", Some(&alex), Some(json!({ "code": enable }))).await;
    assert_eq!(status, StatusCode::OK);

    // Mit 2FA braucht das Verknüpfen zusätzlich einen Code.
    let (status, res) = app
        .call(
            "POST",
            "/auth/oidc/mock/start",
            Some(&bob),
            Some(json!({ "mode": "link", "password": PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (status, res) = app
        .call(
            "POST",
            "/auth/oidc/mock/start",
            Some(&alex),
            Some(json!({ "mode": "link", "password": PASSWORD })),
        )
        .await;
    assert_eq!((status, res["error"].as_str()), (StatusCode::FORBIDDEN, Some("mfa_code_required")));

    let (location, headers) = flow(&app, &mock, "login", None, "alex-sub").await;
    assert!(session_from_cookie(&headers).is_none(), "keine Session vor dem zweiten Faktor");
    assert_eq!(fragment(&location, "mfa_methods"), "totp,recovery");
    let mfa_token = fragment(&location, "mfa_token").to_owned();
    let code = format!("{:06}", mfa::totp_at(&secret, mfa::current_step().max(now) + 1));
    let (status, body) = app
        .call(
            "POST",
            "/auth/login/mfa",
            None,
            Some(json!({ "mfa_token": mfa_token, "method": "totp", "code": code })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["username"], "alex");

    // Export enthält die verknüpfte Identität.
    let req = Request::builder()
        .uri("/api/auth/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {alex}"))
        .body(Body::empty())
        .unwrap();
    let (_, export) = app.send(req).await;
    assert_eq!(export["account_security"]["identities"][0]["provider"], "mock");
    assert_eq!(export["account_security"]["identities"][0]["subject"], "alex-sub");
}

#[sqlx::test]
async fn registration_closed_blocks_new_identities(db: PgPool) {
    let (app, mock) = setup(db.clone()).await;
    app.user("admin", None).await;
    sqlx::query("UPDATE instance_settings SET value = '\"closed\"' WHERE key = 'registration_mode'")
        .execute(&db)
        .await
        .unwrap();
    let (location, _) = flow(&app, &mock, "login", None, "neu").await;
    assert_eq!(fragment(&location, "oidc_error"), "registration_closed");
}

#[sqlx::test]
async fn oauth2_userinfo_signup_and_login(db: PgPool) {
    let (app, mock) = setup(db).await;
    // Erster Nutzer, keine Einladung nötig.
    let state = start_with(&app, &mock, "plain", "login", None, "80351110224678912").await;
    let (status, location, _) = callback_with(&app, "plain", &state, Some(&state)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let signup = fragment(&location, "oidc_signup").to_owned();
    let (_, info) =
        app.call("POST", "/auth/oidc/signup", None, Some(json!({ "signup_token": signup }))).await;
    assert_eq!(info["provider_label"], "Plain-OAuth");
    assert_eq!(info["email"], "80351110224678912@idp.example.org", "bestätigt laut Userinfo");
    let (status, body) = app
        .call(
            "POST",
            "/auth/oidc/register",
            None,
            Some(json!({ "signup_token": signup, "username": "discordwolf", "use_email": false })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // Kein Profil vom Anbieter (Benutzername, Avatar), keine Adresse ohne Zustimmung.
    assert_eq!(body["user"]["display_name"], Value::Null);
    assert_eq!(body["user"]["avatar_url"], Value::Null);
    let (_, sec) = app.call("GET", "/auth/security", Some(s(&body["token"])), None).await;
    assert_eq!(sec["email"], Value::Null);
    assert_eq!(sec["identities"][0]["provider"], "plain");

    // Erneute Anmeldung über Userinfo.
    let state = start_with(&app, &mock, "plain", "login", None, "80351110224678912").await;
    let (_, location, headers) = callback_with(&app, "plain", &state, Some(&state)).await;
    assert_eq!(location, "/");
    let session = session_from_cookie(&headers).unwrap();
    let (_, me) = app.call("GET", "/auth/me", Some(&session), None).await;
    assert_eq!(me["username"], "discordwolf");

    // Gleiche ID bei einem anderen Anbieter ist eine andere Identität.
    let (location, _) = flow(&app, &mock, "login", None, "80351110224678912").await;
    assert!(fragment(&location, "oidc_signup").len() > 20);

    // PKCE und state schützen auch den OAuth2-Weg.
    let state = start_with(&app, &mock, "plain", "login", None, "x").await;
    mock.next.lock().unwrap().challenge = "falsch".into();
    let (_, location, _) = callback_with(&app, "plain", &state, Some(&state)).await;
    assert_eq!(fragment(&location, "oidc_error"), "oidc_failed");
    let state = start_with(&app, &mock, "plain", "login", None, "x").await;
    let (_, location, _) = callback_with(&app, "plain", &state, None).await;
    assert_eq!(fragment(&location, "oidc_error"), "invalid_state");
    // state eines anderen Anbieters gilt nicht.
    let state = start_with(&app, &mock, "plain", "login", None, "x").await;
    let (_, location, _) = callback_with(&app, "mock", &state, Some(&state)).await;
    assert_eq!(fragment(&location, "oidc_error"), "invalid_state");
}
