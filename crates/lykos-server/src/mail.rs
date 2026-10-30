//! Mailversand: Bestätigungslinks, Passwort-Reset, 2FA-Codes und Hinweise zu Adressänderungen.
//!
//! Der Versand läuft asynchron im Hintergrund – Handler warten nicht darauf, und Fehler landen nur im
//! Log (ohne Empfängeradresse), nie beim Client. Jede Mail hat einen Text- und einen HTML-Teil
//! (`multipart/alternative`), beide aus den Vorlagen in `templates/mail/` (siehe [Vorlagen](#vorlagen)).
//! Das Wolfslogo ist eingebettet (`multipart/related`, `cid:`) – keine externen Bilder, Schriften,
//! Tracking-Pixel oder Links zu Dritten.

use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, Result};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use minijinja::{
    AutoEscape, Environment, Error, UndefinedBehavior, Value, context, escape_formatter, syntax::SyntaxConfig,
};
use time::Duration;

use crate::config::{Config, MailConfig, SmtpConfig, SmtpTls};

/// Eine ausgehende Mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub text: String,
    /// Verweist auf das Logo per `cid:` ([`LOGO_CID`]); der SMTP-Versand hängt es eingebettet an.
    pub html: String,
}

/// Versandweg für Mails. `send` kehrt sofort zurück.
pub trait Mailer: Send + Sync {
    /// Ist Mailversand eingerichtet? Sonst blendet die Instanz alle E-Mail-Funktionen aus.
    fn enabled(&self) -> bool {
        true
    }

    /// Übergibt eine Mail zum Versand.
    fn send(&self, mail: Mail);
}

/// Mailer laut Konfiguration.
pub fn from_config(config: &MailConfig) -> Result<Arc<dyn Mailer>> {
    Ok(match config {
        MailConfig::Disabled => Arc::new(DisabledMailer),
        MailConfig::Log => {
            tracing::warn!(
                "LYKOS_MAIL=log: Mails werden NICHT verschickt, sondern samt Links und Codes geloggt – nur für die Entwicklung!"
            );
            Arc::new(LogMailer)
        }
        MailConfig::Smtp(smtp) => Arc::new(SmtpMailer::new(smtp)?),
    })
}

/// Kein Mailversand konfiguriert.
pub struct DisabledMailer;

impl Mailer for DisabledMailer {
    fn enabled(&self) -> bool {
        false
    }

    fn send(&self, _mail: Mail) {}
}

/// Entwicklung: schreibt Mails ins Log statt sie zu verschicken.
pub struct LogMailer;

impl Mailer for LogMailer {
    fn send(&self, mail: Mail) {
        // Bewusst mit Adresse und Inhalt: Ohne das wäre der Log-Mailer nutzlos. Nur für Entwicklung.
        tracing::warn!(
            to = %mail.to,
            subject = %mail.subject,
            "[DEV-LOG-MAILER, nicht für den Betrieb] Mail nicht verschickt:\n{}",
            mail.text
        );
    }
}

/// Versand über SMTP (lettre, rustls).
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    pub fn new(cfg: &SmtpConfig) -> Result<Self> {
        crate::install_crypto_provider();
        let mut builder = match cfg.tls {
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)?,
            SmtpTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?,
            SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host),
        };
        if let Some(port) = cfg.port {
            builder = builder.port(port);
        }
        if let Some(user) = &cfg.username {
            builder =
                builder.credentials(Credentials::new(user.clone(), cfg.password.clone().unwrap_or_default()));
        }
        builder = builder.timeout(Some(std::time::Duration::from_secs(20)));
        let from = Mailbox::new(Some("Lykos".into()), cfg.from.parse().context("LYKOS_MAIL_FROM ungültig")?);
        tracing::info!(host = %cfg.host, port = ?cfg.port, tls = ?cfg.tls, "Mailversand: SMTP");
        Ok(Self { transport: builder.build(), from })
    }
}

/// MIME-Aufbau: `multipart/alternative` aus Text und `multipart/related` (HTML + eingebettetes Logo).
fn message(from: &Mailbox, mail: Mail) -> Result<Message> {
    let logo = Attachment::new_inline(LOGO_CID.into())
        .body(LOGO_PNG.to_vec(), "image/png".parse::<ContentType>().context("Content-Type")?);
    let html = MultiPart::related().singlepart(SinglePart::html(mail.html)).singlepart(logo);
    Ok(Message::builder()
        .from(from.clone())
        .to(mail.to.parse().context("Empfängeradresse ungültig")?)
        .subject(mail.subject)
        .multipart(MultiPart::alternative().singlepart(SinglePart::plain(mail.text)).multipart(html))?)
}

impl Mailer for SmtpMailer {
    fn send(&self, mail: Mail) {
        let subject = mail.subject.clone();
        let message = match message(&self.from, mail) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, %subject, "Mail konnte nicht erstellt werden");
                return;
            }
        };
        let transport = self.transport.clone();
        tokio::spawn(async move {
            // Ohne Empfängeradresse loggen (Datenschutz).
            if let Err(e) = transport.send(message).await {
                tracing::error!(error = %e, %subject, "Mailversand fehlgeschlagen");
            }
        });
    }
}

/// Für Tests: sammelt alle Mails, statt sie zu verschicken.
#[derive(Default, Clone)]
pub struct CollectingMailer {
    pub sent: Arc<Mutex<Vec<Mail>>>,
}

impl CollectingMailer {
    /// Alle bisher „verschickten“ Mails (und leert die Liste).
    pub fn take(&self) -> Vec<Mail> {
        std::mem::take(&mut *self.sent.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Mailer for CollectingMailer {
    fn send(&self, mail: Mail) {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).push(mail);
    }
}

// --- Adressen ---------------------------------------------------------------------------------------

/// Höchstlänge einer E-Mail-Adresse (RFC 5321).
pub const EMAIL_MAX_LEN: usize = 254;

/// Grobe Plausibilitätsprüfung: `lokal@domain.tld`, ohne Leer- und Steuerzeichen. Ob die Adresse
/// existiert, zeigt erst der Bestätigungslink.
pub fn is_valid_email(email: &str) -> bool {
    if email.len() > EMAIL_MAX_LEN || email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let Some((local, domain)) = email.rsplit_once('@') else { return false };
    !local.is_empty()
        && local.len() <= 64
        && !local.contains('@')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.contains('.')
        && !domain.contains("..")
        && domain.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        && !email.contains(['<', '>', '"', ',', ';', '(', ')', '[', ']', '\\'])
}

/// `alex@example.org` → `a•••@example.org`: genug zum Wiedererkennen, ohne eine Adresse ganz preiszugeben.
fn mask_email(email: &str) -> String {
    match email.rsplit_once('@') {
        Some((local, domain)) => format!("{}•••@{domain}", local.chars().next().unwrap_or('•')),
        None => "•••".into(),
    }
}

// --- Vorlagen ---------------------------------------------------------------------------------------
//
// Jede Mail besteht aus `templates/mail/<name>.html` und `<name>.txt` (beide erben von `_base.*`),
// gerendert mit minijinja und beim Kompilieren eingebettet. `.html` wird automatisch escaped –
// Nutzerdaten (Name, Instanz, Adressen, Links) gelangen nur als Variablen hinein, nie per `|safe`.
// Unbekannte Variablen sind ein Fehler (`UndefinedBehavior::Strict`); die Tests rendern jede Vorlage.
// Vorschau: `cargo run -p lykos-server --example mail_preview`.

/// Content-ID des eingebetteten Logos (`<img src="cid:lykos-logo">`).
pub const LOGO_CID: &str = "lykos-logo";
/// Wolfslogo als PNG (128 × 128, angezeigt in 40 × 40) – viele Mail-Clients zeigen kein SVG.
pub const LOGO_PNG: &[u8] = include_bytes!("../templates/mail/logo.png");

macro_rules! templates {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../templates/mail/", $name)))),*]
    };
}

const TEMPLATES: &[(&str, &str)] = templates!(
    "_base.html",
    "_base.txt",
    "verify_email.html",
    "verify_email.txt",
    "password_reset.html",
    "password_reset.txt",
    "mfa_code.html",
    "mfa_code.txt",
    "email_changed.html",
    "email_changed.txt",
    "email_removed.html",
    "email_removed.txt",
);

static TEMPLATE_ENV: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_syntax(
        SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .build()
            .unwrap_or_else(|e| panic!("Mail-Vorlagen: Syntax: {e}")),
    );
    // Wie minijinjas HTML-Escaping, aber ohne `/` → `&#x2f;`: Links bleiben im Quelltext lesbar
    // (und für Clients mit schwachen Parsern intakt). `& < > " '` reichen für Text und Attribute in `"…"`.
    env.set_formatter(|out, state, value| {
        if matches!(state.auto_escape(), AutoEscape::Html)
            && !value.is_safe()
            && let Some(s) = value.as_str()
        {
            return out.write_str(&escape_html(s)).map_err(Error::from);
        }
        escape_formatter(out, state, value)
    });
    for (name, source) in TEMPLATES {
        env.add_template(name, source).unwrap_or_else(|e| panic!("Mail-Vorlage {name}: {e:#}"));
    }
    env
});

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Absendende Instanz:Name und öffentliche Adresse (Fußzeile, „Melde dich an“-Links).
#[derive(Debug, Clone, Copy)]
pub struct Instance<'a> {
    pub name: &'a str,
    pub url: &'a str,
}

impl<'a> From<&'a Config> for Instance<'a> {
    fn from(config: &'a Config) -> Self {
        Self { name: &config.instance_name, url: config.public_url.trim_end_matches('/') }
    }
}

/// Was eine Vorlage braucht. Alles, was `_base.*` kennt, wird immer gesetzt (fehlend = `none`).
struct View<'a> {
    template: &'static str,
    subject: String,
    preheader: String,
    username: &'a str,
    /// Knopf mit Link (`url`, Beschriftung) – im Textteil als Klartext-Link.
    action: Option<(&'a str, &'static str)>,
    code: Option<&'a str>,
    valid_for: Option<Duration>,
    valid_note: Option<&'static str>,
    /// Zusätzliche Variablen einzelner Vorlagen.
    extra: Value,
}

fn render(to: &str, instance: Instance<'_>, view: View<'_>) -> Mail {
    let ctx = context! {
        subject => view.subject.as_str(),
        preheader => view.preheader,
        instance => instance.name,
        public_url => instance.url,
        username => view.username,
        action_url => view.action.map(|(url, _)| url),
        action_label => view.action.map(|(_, label)| label),
        code => view.code,
        valid_for => view.valid_for.map(format_duration),
        valid_note => view.valid_note,
        ..view.extra
    };
    // Die Vorlagen sind fest eingebaut und werden in den Tests alle gerendert – ein Fehler hier ist ein
    // Programmierfehler, kein Laufzeitzustand.
    let part = |ext: &str| {
        TEMPLATE_ENV
            .get_template(&format!("{}.{ext}", view.template))
            .and_then(|t| t.render(ctx.clone()))
            .unwrap_or_else(|e| panic!("Mail-Vorlage {}.{ext}: {e:#}", view.template))
    };
    Mail { to: to.into(), subject: view.subject.clone(), text: part("txt"), html: part("html") }
}

/// „30 Minuten“, „24 Stunden“ – für die Ablaufzeit in den Mails.
fn format_duration(d: Duration) -> String {
    let minutes = d.whole_minutes();
    match minutes {
        60 => "1 Stunde".into(),
        m if m > 60 && m % 60 == 0 => format!("{} Stunden", m / 60),
        1 => "1 Minute".into(),
        m => format!("{m} Minuten"),
    }
}

/// Bestätigungslink für eine (neue) E-Mail-Adresse.
pub fn verify_email(
    to: &str,
    instance: Instance<'_>,
    username: &str,
    link: &str,
    valid_for: Duration,
) -> Mail {
    render(
        to,
        instance,
        View {
            template: "verify_email",
            subject: "Lykos: E-Mail-Adresse bestätigen".into(),
            preheader: format!("Ein Klick, und deine Adresse ist für {} bestätigt.", instance.name),
            username,
            action: Some((link, "E-Mail-Adresse bestätigen")),
            code: None,
            valid_for: Some(valid_for),
            valid_note: None,
            extra: context! {},
        },
    )
}

/// Link zum Zurücksetzen des Passworts.
pub fn password_reset(
    to: &str,
    instance: Instance<'_>,
    username: &str,
    link: &str,
    valid_for: Duration,
) -> Mail {
    render(
        to,
        instance,
        View {
            template: "password_reset",
            subject: "Lykos: Passwort zurücksetzen".into(),
            preheader: format!(
                "Lege über den Link ein neues Passwort fest – gültig für {}.",
                format_duration(valid_for)
            ),
            username,
            action: Some((link, "Neues Passwort festlegen")),
            code: None,
            valid_for: Some(valid_for),
            valid_note: Some("nur einmal verwendbar"),
            extra: context! {},
        },
    )
}

/// Einmal-Code für die Zwei-Faktor-Anmeldung bzw. Bestätigung.
pub fn mfa_code(to: &str, instance: Instance<'_>, username: &str, code: &str, valid_for: Duration) -> Mail {
    render(
        to,
        instance,
        View {
            template: "mfa_code",
            subject: format!("Lykos: Dein Code {code}"),
            preheader: format!(
                "Dein Code für {} – gültig für {}.",
                instance.name,
                format_duration(valid_for)
            ),
            username,
            action: None,
            code: Some(code),
            valid_for: Some(valid_for),
            valid_note: Some("nur einmal verwendbar"),
            extra: context! {},
        },
    )
}

/// Hinweis an die bisherige Adresse, wenn eine neue bestätigt wurde. Die neue Adresse erscheint nur
/// maskiert (`a•••@example.org`).
pub fn email_changed(to: &str, instance: Instance<'_>, username: &str, new_email: &str) -> Mail {
    render(
        to,
        instance,
        View {
            template: "email_changed",
            subject: "Lykos: E-Mail-Adresse geändert".into(),
            preheader: format!(
                "Für dein Konto auf {} ist jetzt eine neue Adresse hinterlegt.",
                instance.name
            ),
            username,
            action: None,
            code: None,
            valid_for: None,
            valid_note: None,
            extra: context! { new_email => mask_email(new_email) },
        },
    )
}

/// Hinweis an die bisherige Adresse, wenn sie entfernt wurde.
pub fn email_removed(to: &str, instance: Instance<'_>, username: &str) -> Mail {
    render(
        to,
        instance,
        View {
            template: "email_removed",
            subject: "Lykos: E-Mail-Adresse entfernt".into(),
            preheader: format!(
                "Diese Adresse ist nicht mehr mit deinem Konto auf {} verknüpft.",
                instance.name
            ),
            username,
            action: None,
            code: None,
            valid_for: None,
            valid_note: None,
            extra: context! {},
        },
    )
}

/// Jede Mail-Art einmal mit Beispieldaten – für `examples/mail_preview.rs` und die Tests.
#[doc(hidden)]
pub fn preview_samples() -> Vec<(&'static str, Mail)> {
    let instance = Instance { name: "Lykos Pilot", url: "https://lykos.example.org" };
    let to = "alex@example.org";
    let user = "alex";
    vec![
        (
            "verify_email",
            verify_email(
                to,
                instance,
                user,
                "https://lykos.example.org/verify-email#token=Q2hhbmdlTWVQbGVhc2VJdHNBVG9rZW4",
                Duration::hours(24),
            ),
        ),
        (
            "password_reset",
            password_reset(
                to,
                instance,
                user,
                "https://lykos.example.org/reset-password#token=UmVzZXRNZVBsZWFzZUl0c0FUb2tlbg",
                Duration::minutes(30),
            ),
        ),
        ("mfa_code", mfa_code(to, instance, user, "482913", Duration::minutes(10))),
        ("email_changed", email_changed(to, instance, user, "alex.neu@example.net")),
        ("email_removed", email_removed(to, instance, user)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTANCE: Instance<'static> = Instance { name: "Testinstanz", url: "https://lykos.test" };

    #[test]
    fn email_validation() {
        assert!(is_valid_email("alex@example.org"));
        assert!(is_valid_email("a.b+lykos@sub.example.de"));
        assert!(is_valid_email("ümlaut@bücher.de"));
        for bad in
            ["", "alex", "@example.org", "alex@", "alex@localhost", "a b@example.org", "a@b..de", "a@.b.de"]
        {
            assert!(!is_valid_email(bad), "{bad}");
        }
        assert!(!is_valid_email("\"x\"<a@b.de>"));
        assert!(!is_valid_email(&format!("{}@example.org", "a".repeat(65))));
    }

    #[test]
    fn templates_escape_html() {
        let evil = Instance { name: "<Test & Co>", url: "https://x\"onmouseover=\"alert(1)" };
        let m = verify_email(
            "a@b.de",
            evil,
            "<script>alert(1)</script>",
            "https://x/verify-email?token=a&b\"><img src=x>",
            Duration::hours(24),
        );
        // Textteil: unverändert (kein HTML).
        assert!(m.text.contains("https://x/verify-email?token=a&b\"><img src=x>"));
        assert!(m.text.contains("Hallo <script>alert(1)</script>,"));
        // HTML-Teil: alles escaped, nichts davon als Markup.
        assert!(m.html.contains("&lt;Test &amp; Co&gt;"));
        assert!(m.html.contains("&lt;script&gt;"));
        assert!(!m.html.contains("<script"));
        assert!(!m.html.contains("<img src=x"));
        assert!(!m.html.contains("\"onmouseover"));
        assert!(m.html.contains("token=a&amp;b&quot;&gt;"));
        // Einziges Bild ist das eingebettete Logo.
        assert_eq!(m.html.matches("<img").count(), 1);
        assert!(m.html.contains("src=\"cid:lykos-logo\""));

        let m = email_changed("a@b.de", INSTANCE, "x", "<b>@evil.de");
        assert!(m.html.contains("&lt;•••@evil.de"));
        assert!(!m.html.contains("<b>"));
    }

    #[test]
    fn all_templates_render_completely() {
        let samples = preview_samples();
        assert_eq!(samples.len() * 2 + 2, TEMPLATES.len(), "jede Vorlage braucht ein Beispiel");
        for (name, m) in samples {
            for part in [&m.html, &m.text] {
                assert!(!part.contains("{{") && !part.contains("{%") && !part.contains("}}"), "{name}");
                assert!(part.contains("Hallo alex,"), "{name}");
                assert!(part.contains("Lykos Pilot"), "{name}: Instanzname");
                assert!(part.contains("https://lykos.example.org"), "{name}: Instanz-URL");
                assert!(part.contains("Powered by Zeasy Software"), "{name}");
                assert!(
                    part.contains("Warst du das nicht?") || part.contains("Das warst nicht du?"),
                    "{name}"
                );
            }
            assert!(!m.text.contains("none"), "{name}: leerer Platzhalter");
            assert!(
                ![">none<", "\"none\"", " none "].iter().any(|n| m.html.contains(n)),
                "{name}: leerer Platzhalter"
            );
            assert!(m.html.starts_with("<!DOCTYPE html>"), "{name}");
            assert!(m.html.contains("<title>Lykos: "), "{name}");
            assert!(m.html.contains("src=\"cid:lykos-logo\""), "{name}");
            assert!(!m.html.contains("http://"), "{name}: keine unverschlüsselten/externen Ressourcen");
            assert!(!m.text.contains("<a ") && !m.text.contains("<p") && !m.text.contains("&amp;"), "{name}");
            assert!(m.subject.starts_with("Lykos: "), "{name}");
        }
    }

    #[test]
    fn links_and_codes_stand_alone() {
        let link = "https://lykos.test/reset-password?token=abc";
        let m = password_reset("a@b.de", INSTANCE, "alex", link, Duration::minutes(30));
        // Eigene Zeile im Textteil (Mail-Clients machen sie klickbar, Tests lesen sie aus).
        assert!(m.text.lines().any(|l| l == link));
        assert!(m.text.contains("Gültig für 30 Minuten · nur einmal verwendbar."));
        // HTML: Knopf und Klartext-Link als Fallback.
        assert_eq!(m.html.matches(&format!("href=\"{link}\"")).count(), 2);
        assert!(m.html.contains("Neues Passwort festlegen</a>"));

        let m = mfa_code("a@b.de", INSTANCE, "alex", "012345", Duration::minutes(10));
        assert!(m.text.lines().any(|l| l.trim() == "012345"));
        assert!(m.html.contains(">012345</td>"));
        assert!(!m.html.contains("Der Knopf funktioniert nicht?"));
        assert_eq!(m.subject, "Lykos: Dein Code 012345");
    }

    /// Schnappschuss des kompletten Textteils – fällt auf, wenn Leerzeilen/Umbrüche verrutschen.
    #[test]
    fn mfa_text_snapshot() {
        let m = mfa_code("a@b.de", INSTANCE, "alex", "482913", Duration::minutes(10));
        let expected = "\
Dein Bestätigungscode

Hallo alex,

mit diesem Code bestätigst du deine Anmeldung bzw. Änderung auf Testinstanz. Gib ihn niemandem weiter – niemand von Testinstanz wird dich je danach fragen.

    482913

Gültig für 10 Minuten · nur einmal verwendbar.

Warst du das nicht?

Dann kennt womöglich jemand dein Passwort. Melde dich an und ändere es bitte sofort.

-- 
Diese Mail wurde automatisch von Testinstanz (https://lykos.test) verschickt – bitte antworte nicht darauf.
Lykos · Powered by Zeasy Software";
        assert_eq!(m.text.trim_end(), expected);
    }

    #[test]
    fn helpers() {
        assert_eq!(format_duration(Duration::minutes(10)), "10 Minuten");
        assert_eq!(format_duration(Duration::minutes(1)), "1 Minute");
        assert_eq!(format_duration(Duration::minutes(90)), "90 Minuten");
        assert_eq!(format_duration(Duration::hours(1)), "1 Stunde");
        assert_eq!(format_duration(Duration::hours(24)), "24 Stunden");
        assert_eq!(mask_email("alex@example.org"), "a•••@example.org");
        assert_eq!(mask_email("ü@b.de"), "ü•••@b.de");
        assert_eq!(mask_email("kaputt"), "•••");
    }

    #[test]
    fn mime_structure() {
        let from = Mailbox::new(Some("Lykos".into()), "lykos@example.org".parse().unwrap());
        let (_, mail) = preview_samples().remove(0);
        let raw = String::from_utf8(message(&from, mail).unwrap().formatted()).unwrap();
        let alt = raw.find("multipart/alternative").expect("alternative");
        let plain = raw.find("text/plain").expect("Textteil");
        let related = raw.find("multipart/related").expect("related");
        let html = raw.find("text/html").expect("HTML-Teil");
        let png = raw.find("image/png").expect("Logo");
        assert!(alt < plain && plain < related && related < html && html < png, "{raw}");
        assert!(raw.contains("Content-ID: <lykos-logo>"));
        assert!(raw.contains("Content-Disposition: inline"));
    }
}
