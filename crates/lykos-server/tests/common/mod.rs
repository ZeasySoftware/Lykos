//! Gemeinsame Helfer für Integrationstests. Einbinden mit `mod common;`.
//! Tests laufen gegen eine echte Postgres-Datenbank (`DATABASE_URL`); `sqlx::test`
//! legt pro Test eine frische Datenbank an und führt die Migrationen aus.
#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use lykos_server::{config::Config, mail::CollectingMailer};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

pub const PASSWORD: &str = "ein-sicheres-passwort";

pub struct TestApp {
    pub router: Router,
    pub db: PgPool,
}

impl TestApp {
    pub fn new(db: PgPool) -> Self {
        Self::with_config(db, Config::for_tests())
    }

    pub fn with_config(db: PgPool, config: Config) -> Self {
        let router = lykos_server::app(lykos_server::state(db.clone(), config)).unwrap();
        Self { router, db }
    }

    /// Mit Mailversand: Mails werden gesammelt statt verschickt.
    pub fn with_mailer(db: PgPool, config: Config) -> (Self, CollectingMailer) {
        let mailer = CollectingMailer::default();
        let mut state = lykos_server::state(db.clone(), config);
        state.mailer = Arc::new(mailer.clone());
        (Self { router: lykos_server::app(state).unwrap(), db }, mailer)
    }

    /// Wie `send`, aber mit Antwort-Headern (z. B. `Location`, `Set-Cookie`).
    pub async fn send_full(&self, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, json)
    }

    /// JSON-Request an `/api{path}`, optional mit Bearer-Token.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(format!("/api{path}"));
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let req = match body {
            Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        self.send(req).await
    }

    /// Beliebigen Request senden (z. B. Multipart); Antwort als JSON, falls vorhanden.
    pub async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, json)
    }

    pub async fn register(&self, username: &str, invite: Option<&str>) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/auth/register",
            None,
            Some(json!({ "username": username, "password": PASSWORD, "invite": invite })),
        )
        .await
    }

    /// Registriert und gibt das Token zurück (erwartet Erfolg).
    pub async fn user(&self, username: &str, invite: Option<&str>) -> String {
        let (status, body) = self.register(username, invite).await;
        assert_eq!(status, StatusCode::CREATED, "Registrierung {username}: {body}");
        body["token"].as_str().unwrap().to_owned()
    }

    /// ID des angemeldeten Nutzers.
    pub async fn user_id(&self, token: &str) -> String {
        let (_, me) = self.call("GET", "/auth/me", Some(token), None).await;
        s(&me["id"]).to_owned()
    }

    /// Legt eine Community an.
    pub async fn community(&self, token: &str, name: &str) -> TestCommunity {
        let (status, detail) =
            self.call("POST", "/communities", Some(token), Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED, "Community anlegen: {detail}");
        let channel = |kind: &str| {
            s(&detail["channels"].as_array().unwrap().iter().find(|c| c["kind"] == kind).unwrap()["id"])
                .to_owned()
        };
        TestCommunity {
            id: s(&detail["community"]["id"]).to_owned(),
            text_channel: channel("text"),
            voice_channel: channel("voice"),
            default_role: s(&detail["roles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["is_default"] == true)
                .unwrap()["id"])
            .to_owned(),
        }
    }

    /// Erstellt eine Einladung (als `owner`) und registriert damit `username`; gibt dessen Token zurück.
    pub async fn invited_user(&self, owner: &str, community: &TestCommunity, username: &str) -> String {
        let (_, invite) = self
            .call("POST", &format!("/communities/{}/invites", community.id), Some(owner), Some(json!({})))
            .await;
        self.user(username, Some(s(&invite["code"]))).await
    }
}

pub struct TestCommunity {
    pub id: String,
    pub text_channel: String,
    pub voice_channel: String,
    pub default_role: String,
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}
