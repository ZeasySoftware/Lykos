//! Fehler- und Vorschlagsmeldungen (Paket K): Datenobjekte, Prüfung der Eingaben, Bereinigung der
//! technischen Infos und die optionale Weiterleitung als Gitea-Issue.
//!
//! Sichtbarkeit: Fehlerberichte und „Sonstiges“ sehen nur Melder und Instanz-Admins. Vorschläge sind
//! für alle angemeldeten Nutzer der Instanz sichtbar – dann aber ohne Melder, technische Infos,
//! Screenshot und interne Notizen.
//!
//! Datenschutz: Technische Infos schickt der Client nur mit Opt-in. Der Server übernimmt davon nur
//! bekannte Felder mit begrenzter Länge und schwärzt zusätzlich alles, was nach Token oder ID aussieht.

use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::FeedbackGiteaConfig,
    error::{ApiError, ApiResult},
    models::UserPublic,
};

/// Meldungstypen: Fehler, Vorschlag, Sonstiges.
pub const KINDS: &[&str] = &["bug", "suggestion", "other"];
/// Bearbeitungsstände.
pub const STATUSES: &[&str] = &["neu", "in_arbeit", "erledigt", "abgelehnt", "duplikat"];

/// Titel: 3–150 Zeichen.
pub const TITLE_MIN_CHARS: usize = 3;
pub const TITLE_MAX_CHARS: usize = 150;
/// Beschreibung (Markdown): 1–10 000 Zeichen.
pub const DESCRIPTION_MAX_CHARS: usize = 10_000;
/// Antwort eines Admins: 1–4000 Zeichen.
pub const REPLY_MAX_CHARS: usize = 4000;
/// Meldungen pro Nutzer und Stunde.
pub const PER_HOUR: i64 = 10;
/// Höchstgröße eines Screenshots.
pub const SCREENSHOT_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Größte zulässige Kantenlänge eines Screenshots.
pub const SCREENSHOT_MAX_SIDE: i32 = 8192;
/// So lange nach dem Absenden kann der Melder noch einen Screenshot anhängen.
pub const SCREENSHOT_WINDOW_SECS: i64 = 60 * 60;

/// Grenzen der technischen Infos.
const TECH_SHORT_MAX: usize = 64;
const TECH_ERRORS_MAX: usize = 50;
const TECH_ERROR_MAX_CHARS: usize = 500;

// --- Datenobjekte -------------------------------------------------------------

/// Antwort oder interne Notiz eines Admins.
#[derive(Debug, Clone, Serialize)]
pub struct FeedbackReply {
    pub id: Uuid,
    pub author: Option<UserPublic>,
    pub body: String,
    /// Nur für Admins sichtbar (Melder und Community sehen interne Notizen nie).
    pub internal: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Eine Meldung, wie sie die API ausliefert. Welche Felder gefüllt sind, hängt vom Betrachter ab
/// (siehe `View`): Felder für Melder/Admins fehlen in der öffentlichen Ansicht ganz.
#[derive(Debug, Clone, Serialize)]
pub struct Feedback {
    pub id: Uuid,
    /// Meldungsnummer („#12“).
    pub number: i64,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub vote_count: i64,
    /// Hat der Betrachter (für Vorschläge) schon zugestimmt?
    pub voted: bool,
    /// Ist der Betrachter der Melder?
    pub mine: bool,
    /// Antworten (für Admins inklusive interner Notizen), älteste zuerst.
    pub replies: Vec<FeedbackReply>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Melder und Admins: technische Infos (`null` ohne Opt-in).
    #[serde(skip_serializing_if = "Hidden::is_hidden")]
    pub tech_info: Hidden<Option<Value>>,
    /// Melder und Admins: Pfad des Screenshots (`null` = keiner).
    #[serde(skip_serializing_if = "Hidden::is_hidden")]
    pub screenshot_url: Hidden<Option<String>>,
    /// Nur Admins: Melder (`null` = Konto gelöscht).
    #[serde(skip_serializing_if = "Hidden::is_hidden")]
    pub reporter: Hidden<Option<UserPublic>>,
    /// Nur Admins: Link auf das Gitea-Issue.
    #[serde(skip_serializing_if = "Hidden::is_hidden")]
    pub issue_url: Hidden<Option<String>>,
}

/// Feld, das je nach Betrachter ganz weggelassen wird (statt `null`).
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct Hidden<T>(pub Option<T>);

impl<T> Hidden<T> {
    pub fn is_hidden(&self) -> bool {
        self.0.is_none()
    }
}

/// Wer eine Meldung ansieht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Andere Nutzer (nur Vorschläge).
    Public,
    /// Der Melder selbst.
    Reporter,
    /// Instanz-Admin.
    Admin,
}

#[derive(Debug, FromRow)]
pub struct Row {
    pub id: Uuid,
    pub number: i64,
    pub reporter_id: Option<Uuid>,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub tech_info: Option<Value>,
    pub screenshot_id: Option<Uuid>,
    pub screenshot_content_type: Option<String>,
    pub issue_url: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub vote_count: i64,
    pub voted: bool,
    pub reporter_username: Option<String>,
    pub reporter_display_name: Option<String>,
    pub reporter_avatar_id: Option<Uuid>,
}

/// `SELECT … FROM feedback f` mit allen Spalten für `Row`; `$1` ist der Betrachter (für `voted`).
/// Als Makro, damit Abfragen per `concat!` statische SQL-Strings bleiben.
#[macro_export]
macro_rules! feedback_select {
    () => {
        "SELECT f.id, f.number, f.reporter_id, f.kind, f.status, f.title, f.description,
            f.tech_info, f.screenshot_id, f.screenshot_content_type, f.issue_url, f.created_at, f.updated_at,
            (SELECT count(*) FROM feedback_votes v WHERE v.feedback_id = f.id) AS vote_count,
            EXISTS (SELECT 1 FROM feedback_votes v WHERE v.feedback_id = f.id AND v.user_id = $1) AS voted,
            u.username AS reporter_username, u.display_name AS reporter_display_name,
            u.avatar_id AS reporter_avatar_id
        FROM feedback f LEFT JOIN users u ON u.id = f.reporter_id"
    };
}

impl Row {
    /// Darf `viewer` (mit Admin-Status `is_admin`) die Meldung sehen, und in welcher Ansicht?
    /// `None` = nicht sichtbar (→ `404`, Existenz nicht verraten).
    pub fn view_for(&self, viewer: Uuid, is_admin: bool) -> Option<View> {
        if is_admin {
            Some(View::Admin)
        } else if self.reporter_id == Some(viewer) {
            Some(View::Reporter)
        } else if self.kind == "suggestion" {
            Some(View::Public)
        } else {
            None
        }
    }

    pub fn into_feedback(self, viewer: Uuid, view: View, replies: Vec<FeedbackReply>) -> Feedback {
        let owner_or_admin = matches!(view, View::Reporter | View::Admin);
        let admin = view == View::Admin;
        let screenshot_url = self.screenshot_id.map(|_| format!("/api/feedback/{}/screenshot", self.id));
        let reporter = match (self.reporter_id, self.reporter_username) {
            (Some(id), Some(username)) => {
                Some(UserPublic::new(id, username, self.reporter_display_name, self.reporter_avatar_id))
            }
            _ => None,
        };
        Feedback {
            id: self.id,
            number: self.number,
            kind: self.kind,
            status: self.status,
            title: self.title,
            description: self.description,
            vote_count: self.vote_count,
            voted: self.voted,
            mine: self.reporter_id == Some(viewer),
            replies: replies.into_iter().filter(|r| admin || !r.internal).collect(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            tech_info: Hidden(owner_or_admin.then_some(self.tech_info)),
            screenshot_url: Hidden(owner_or_admin.then_some(screenshot_url)),
            reporter: Hidden(admin.then_some(reporter)),
            issue_url: Hidden(admin.then_some(self.issue_url)),
        }
    }
}

#[derive(FromRow)]
struct ReplyRow {
    id: Uuid,
    feedback_id: Uuid,
    author_id: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    body: String,
    internal: bool,
    created_at: OffsetDateTime,
}

/// Antworten zu mehreren Meldungen (`include_internal` nur für Admins), je Meldung älteste zuerst.
pub async fn replies_for(
    db: impl PgExecutor<'_>,
    ids: &[Uuid],
    include_internal: bool,
) -> ApiResult<Vec<(Uuid, FeedbackReply)>> {
    let rows: Vec<ReplyRow> = sqlx::query_as(
        "SELECT r.id, r.feedback_id, r.author_id, u.username, u.display_name, u.avatar_id, r.body, r.internal,
                r.created_at
         FROM feedback_replies r LEFT JOIN users u ON u.id = r.author_id
         WHERE r.feedback_id = ANY($1) AND ($2 OR NOT r.internal)
         ORDER BY r.id",
    )
    .bind(ids)
    .bind(include_internal)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let author = match (r.author_id, r.username) {
                (Some(id), Some(username)) => {
                    Some(UserPublic::new(id, username, r.display_name, r.avatar_id))
                }
                _ => None,
            };
            (
                r.feedback_id,
                FeedbackReply {
                    id: r.id,
                    author,
                    body: r.body,
                    internal: r.internal,
                    created_at: r.created_at,
                },
            )
        })
        .collect())
}

// --- Eingaben -----------------------------------------------------------------

/// Freitext: ohne Steuerzeichen (außer Zeilenumbruch/Tab), getrimmt, Länge `min..=max` Zeichen.
pub fn clean_text(text: &str, min: usize, max: usize, code: &'static str) -> ApiResult<String> {
    let cleaned: String = text.chars().filter(|c| matches!(c, '\n' | '\t') || !c.is_control()).collect();
    let trimmed = cleaned.trim();
    let len = trimmed.chars().count();
    if len < min || len > max {
        return Err(ApiError::BadRequest(code));
    }
    Ok(trimmed.to_owned())
}

/// Titel: eine Zeile.
pub fn clean_title(title: &str) -> ApiResult<String> {
    let one_line = title.replace(['\n', '\r', '\t'], " ");
    clean_text(&one_line, TITLE_MIN_CHARS, TITLE_MAX_CHARS, "invalid_title")
}

pub fn validate_kind(kind: &str) -> ApiResult<()> {
    if KINDS.contains(&kind) { Ok(()) } else { Err(ApiError::BadRequest("invalid_kind")) }
}

pub fn validate_status(status: &str) -> ApiResult<()> {
    if STATUSES.contains(&status) { Ok(()) } else { Err(ApiError::BadRequest("invalid_status")) }
}

// --- Technische Infos ---------------------------------------------------------

/// Technische Infos, wie der Client sie schickt. Unbekannte Felder werden verworfen.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TechInfoIn {
    app_version: Option<String>,
    platform: Option<String>,
    os: Option<String>,
    window: Option<WindowSize>,
    view: Option<String>,
    errors: Vec<ClientErrorIn>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
struct WindowSize {
    width: u32,
    height: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ClientErrorIn {
    at: Option<String>,
    message: String,
}

/// Bereinigt die technischen Infos des Clients: nur bekannte Felder, Längen begrenzt, Tokens und IDs
/// geschwärzt. `invalid_tech_info`, wenn es kein passendes Objekt ist.
pub fn sanitize_tech_info(raw: &Value) -> ApiResult<Value> {
    let info: TechInfoIn =
        serde_json::from_value(raw.clone()).map_err(|_| ApiError::BadRequest("invalid_tech_info"))?;
    let short =
        |s: Option<String>| s.map(|s| truncate(&redact(&s), TECH_SHORT_MAX)).filter(|s| !s.is_empty());
    let platform = info.platform.filter(|p| matches!(p.as_str(), "web" | "desktop"));
    let window = info.window.filter(|w| w.width <= 100_000 && w.height <= 100_000);
    let errors: Vec<Value> = info
        .errors
        .into_iter()
        .rev()
        .take(TECH_ERRORS_MAX)
        .rev()
        .filter_map(|e| {
            let message = truncate(&redact(&e.message), TECH_ERROR_MAX_CHARS);
            if message.is_empty() {
                return None;
            }
            // Zeitstempel nur, wenn es wirklich einer ist.
            let at = e
                .at
                .filter(|a| OffsetDateTime::parse(a, &time::format_description::well_known::Rfc3339).is_ok());
            Some(json!({ "at": at, "message": message }))
        })
        .collect();
    Ok(json!({
        "app_version": short(info.app_version),
        "platform": platform,
        "os": short(info.os),
        "window": window,
        "view": short(info.view),
        "errors": errors,
    }))
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().filter(|c| *c == '\n' || !c.is_control()).take(max).collect::<String>().trim().to_owned()
}

/// Schwärzt alles, was nach Geheimnis oder Kennung aussieht: UUIDs → `<id>`, lange Zeichenketten aus
/// Buchstaben/Ziffern/`-_` (Session-Tokens, JWT-Teile, Hashes) → `<redacted>`; `Bearer …` immer.
pub fn redact(text: &str) -> String {
    fn is_token_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    }
    fn is_uuid(w: &str) -> bool {
        w.len() == 36
            && w.char_indices()
                .all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
    }

    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut after_bearer = false;
    let flush = |word: &mut String, out: &mut String, after_bearer: &mut bool| {
        if word.is_empty() {
            return;
        }
        if *after_bearer {
            out.push_str("<redacted>");
            *after_bearer = false;
        } else if is_uuid(word) {
            out.push_str("<id>");
        } else if word.chars().count() >= 24 && word.chars().any(|c| c.is_ascii_digit()) {
            out.push_str("<redacted>");
        } else {
            *after_bearer = word.eq_ignore_ascii_case("bearer");
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if is_token_char(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &mut after_bearer);
            if !(after_bearer && c == ' ') {
                after_bearer = false;
            }
            out.push(c);
        }
    }
    flush(&mut word, &mut out, &mut after_bearer);
    out
}

// --- Gitea --------------------------------------------------------------------

/// Text als Markdown-Codeblock: Der Zaun ist länger als jede Backtick-Folge im Text, damit nichts
/// ausbrechen kann. Im Codeblock rendert Gitea weder Bilder (`![…](…)`, externe Tracking-Pixel) noch
/// Links, @-Erwähnungen (Benachrichtigungen an Fremde) oder Issue-Verweise.
fn code_block(text: &str, lang: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}{lang}\n{text}\n{fence}")
}

/// Einzeiliger Nutzertext (Titel) ohne @-Erwähnungen: Gitea benachrichtigt sonst erwähnte Konten.
fn defuse_mentions(text: &str) -> String {
    text.replace('@', "@\u{200b}")
}

/// Gitea-Issue zu einer Meldung. Enthält bewusst keinen Nutzernamen, nur die Meldungsnummer. Die
/// Beschreibung kommt unverändert, aber als Codeblock (`code_block`) hinein.
pub fn issue_content(feedback: &Row) -> (String, String) {
    let kind = match feedback.kind.as_str() {
        "bug" => "Fehler",
        "suggestion" => "Vorschlag",
        _ => "Sonstiges",
    };
    let title = format!("[Feedback #{}] {}", feedback.number, defuse_mentions(&feedback.title));
    let mut body = format!(
        "{}\n\n---\n**Typ:** {kind} · **Lykos-Meldung:** #{}\n",
        code_block(&feedback.description, "text"),
        feedback.number
    );
    if feedback.screenshot_id.is_some() {
        body.push_str("\nEin Screenshot liegt in Lykos (nicht übertragen).\n");
    }
    if let Some(info) = &feedback.tech_info {
        let pretty = serde_json::to_string_pretty(info).unwrap_or_default();
        body.push_str(&format!(
            "\n<details>\n<summary>Technische Infos</summary>\n\n{}\n\n</details>\n",
            code_block(&pretty, "json")
        ));
    }
    (title, body)
}

#[derive(Deserialize)]
struct GiteaLabel {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct GiteaIssue {
    html_url: String,
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::install_crypto_provider();
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("lykos-server/", env!("CARGO_PKG_VERSION")))
            // Kein Umleiten: das Token soll nur an die konfigurierte Adresse gehen.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("HTTP-Client")
    })
}

/// Label-Namen für einen Meldungstyp (verwendet werden nur die, die es im Repo gibt).
fn wanted_labels(kind: &str) -> &'static [&'static str] {
    match kind {
        "bug" => &["feedback", "bug"],
        "suggestion" => &["feedback", "enhancement"],
        _ => &["feedback"],
    }
}

/// Legt ein Issue an und gibt dessen Web-Adresse zurück. Labels sind optional: Fehlt eines oder
/// ist die Label-Liste nicht lesbar, wird das Issue ohne sie angelegt.
pub async fn create_issue(
    config: &FeedbackGiteaConfig,
    kind: &str,
    title: &str,
    body: &str,
) -> Result<String> {
    let auth = format!("token {}", config.token);
    let labels = match http()
        .get(format!("{}/labels?limit=50", config.api_url))
        .header(reqwest::header::AUTHORIZATION, &auth)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => res.json::<Vec<GiteaLabel>>().await.unwrap_or_default(),
        Ok(res) => {
            tracing::warn!(status = %res.status(), "Gitea-Labels nicht lesbar");
            vec![]
        }
        Err(e) => {
            tracing::warn!(error = %e, "Gitea-Labels nicht lesbar");
            vec![]
        }
    };
    let wanted = wanted_labels(kind);
    let label_ids: Vec<i64> = labels
        .iter()
        .filter(|l| wanted.iter().any(|w| l.name.eq_ignore_ascii_case(w)))
        .map(|l| l.id)
        .collect();

    let res = http()
        .post(format!("{}/issues", config.api_url))
        .header(reqwest::header::AUTHORIZATION, &auth)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&json!({ "title": title, "body": body, "labels": label_ids }))
        .send()
        .await
        .context("Gitea nicht erreichbar")?;
    if !res.status().is_success() {
        bail!("Gitea: Issue anlegen fehlgeschlagen ({})", res.status());
    }
    let issue: GiteaIssue = res.json().await.context("Gitea: ungültige Antwort")?;
    if !(issue.html_url.starts_with("https://") || issue.html_url.starts_with("http://")) {
        bail!("Gitea: ungültige Issue-URL");
    }
    Ok(issue.html_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_hides_tokens_and_ids() {
        assert_eq!(
            redact("GET /api/channels/0192f3a4-5b6c-7d8e-9f01-23456789abcd/messages failed"),
            "GET /api/channels/<id>/messages failed"
        );
        assert_eq!(redact("Authorization: Bearer abc.def"), "Authorization: Bearer <redacted>.def");
        assert_eq!(redact("token=Zx8k2Lq9Vb7Nm3Pw5Rt1Yu6Io4As0Df"), "token=<redacted>");
        // Normale Wörter und kurze Zahlen bleiben.
        assert_eq!(
            redact("TypeError: Cannot read properties of undefined (reading 'x') at line 42"),
            "TypeError: Cannot read properties of undefined (reading 'x') at line 42"
        );
        // Lange Wörter ohne Ziffern (z. B. Komponentennamen) bleiben ebenfalls.
        assert_eq!(redact("VeryLongComponentNameWithoutDigits"), "VeryLongComponentNameWithoutDigits");
    }

    #[test]
    fn tech_info_keeps_only_known_fields() {
        let raw = json!({
            "app_version": "0.3.0",
            "platform": "desktop",
            "os": "Windows",
            "window": { "width": 1280, "height": 800 },
            "view": "/channels/0192f3a4-5b6c-7d8e-9f01-23456789abcd",
            "errors": [
                { "at": "2026-10-29T12:00:00Z", "message": "boom" },
                { "at": "gestern", "message": "  " },
            ],
            "cookies": "lykos_session=geheim",
            "user": { "name": "alex" },
        });
        let clean = sanitize_tech_info(&raw).unwrap();
        assert_eq!(
            clean,
            json!({
                "app_version": "0.3.0",
                "platform": "desktop",
                "os": "Windows",
                "window": { "width": 1280, "height": 800 },
                "view": "/channels/<id>",
                "errors": [{ "at": "2026-10-29T12:00:00Z", "message": "boom" }],
            })
        );
        // Unbekannte Plattform fällt weg, zu viele Fehler werden gekappt (die neuesten bleiben).
        let many: Vec<Value> = (0..80).map(|i| json!({ "message": format!("Fehler {i}") })).collect();
        let clean = sanitize_tech_info(&json!({ "platform": "toaster", "errors": many })).unwrap();
        assert_eq!(clean["platform"], Value::Null);
        let errors = clean["errors"].as_array().unwrap();
        assert_eq!(errors.len(), TECH_ERRORS_MAX);
        assert_eq!(errors[0]["message"], "Fehler 30");
        assert!(sanitize_tech_info(&json!("kein Objekt")).is_err());
        assert!(sanitize_tech_info(&json!({ "errors": "x" })).is_err());
    }

    #[test]
    fn texts_are_checked() {
        assert_eq!(clean_title("  Absturz\nbeim Start ").unwrap(), "Absturz beim Start");
        assert!(clean_title("ab").is_err());
        assert!(clean_title(&"x".repeat(151)).is_err());
        assert_eq!(clean_text(" a\u{0}b\n ", 1, 10, "x").unwrap(), "ab");
        assert!(validate_kind("bug").is_ok());
        assert!(validate_kind("feature").is_err());
        assert!(validate_status("duplikat").is_ok());
        assert!(validate_status("open").is_err());
    }

    #[test]
    fn issue_text_is_defused() {
        // Codeblock: Zaun immer länger als jede Backtick-Folge im Text.
        assert_eq!(code_block("hi", "text"), "```text\nhi\n```");
        let tricky = "a\n```\n![x](https://evil.example/p.png) @admin\n````";
        let block = code_block(tricky, "text");
        assert!(block.starts_with("`````text\n") && block.ends_with("\n`````"), "{block}");
        assert_eq!(defuse_mentions("Hilfe @admin"), "Hilfe @\u{200b}admin");
    }
}
