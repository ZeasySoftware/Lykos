//! Sicherheits-Header für alle Antworten (Web-Client und API).
//!
//! Die Content-Security-Policy erlaubt nur, was der Web-Client tatsächlich braucht:
//!
//! - Skripte, Styles, Schriften, Manifest nur vom eigenen Origin (der Build enthält keine Inline-Skripte;
//!   `style-src 'unsafe-inline'` wegen React-Inline-Styles (`style={…}`)).
//! - `connect-src`: eigene API und Gateway-WebSocket (`'self'`, zusätzlich explizit `ws(s)://` der
//!   `public_url` für ältere Browser) sowie die LiveKit-Signalisierung (`wss://…` und `https://…` desselben
//!   Hosts – das LiveKit-SDK prüft Verbindungsfehler per HTTP nach).
//! - Bilder vom eigenen Origin (Avatare, Emojis, Anhänge) sowie `data:`/`blob:`
//!   (Vorschauen, zugeschnittene Avatare, Screenshots); Audio/Video vom eigenen Origin und `blob:`.
//! - `worker-src 'self' blob:` für Worker; das AudioWorklet (Mikrofon-Pegel/Gate) fällt unter `script-src`
//!   und kommt als eigene Datei vom eigenen Origin.
//! - Keine Plugins, kein `<base>`-Umbiegen, keine fremden Formular-Ziele, nicht einbettbar.
//!
//! Header, die eine Route selbst setzt (z. B. `sandbox` für Anhänge und GIFs), bleiben unverändert.
//! HSTS nur, wenn die Instanz unter `https://` erreichbar ist (`LYKOS_PUBLIC_URL`).

use std::sync::Arc;

use axum::{
    http::{HeaderName, HeaderValue, header},
    response::Response,
};

use crate::config::Config;

/// Header samt Wert, die jede Antwort bekommt (sofern die Route sie nicht selbst gesetzt hat).
#[derive(Clone)]
pub struct SecurityHeaders(Arc<Vec<(HeaderName, HeaderValue)>>);

impl SecurityHeaders {
    pub fn from_config(config: &Config) -> anyhow::Result<Self> {
        let mut headers = vec![
            (header::CONTENT_SECURITY_POLICY, HeaderValue::from_str(&content_security_policy(config))?),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
            (header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
            (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ];
        if config.public_url.starts_with("https://") {
            headers.push((header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000")));
        }
        Ok(Self(Arc::new(headers)))
    }

    /// Ergänzt fehlende Header (für `axum::middleware::map_response`).
    pub fn apply(&self, mut res: Response) -> Response {
        let h = res.headers_mut();
        for (name, value) in self.0.iter() {
            if !h.contains_key(name) {
                h.insert(name.clone(), value.clone());
            }
        }
        res
    }
}

/// `scheme://host[:port]` einer URL (ohne Pfad, Query, Zugangsdaten); `None` bei unbrauchbarer Eingabe.
fn origin_parts(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let valid = !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    valid.then_some((scheme, host))
}

/// WebSocket- und HTTP-Variante eines Origins (`wss://x` + `https://x` bzw. `ws://x` + `http://x`).
fn ws_and_http(url: &str) -> Vec<String> {
    let Some((scheme, host)) = origin_parts(url) else {
        return Vec::new();
    };
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "wss" | "https" => true,
        "ws" | "http" => false,
        _ => return Vec::new(),
    };
    if secure {
        vec![format!("wss://{host}"), format!("https://{host}")]
    } else {
        vec![format!("ws://{host}"), format!("http://{host}")]
    }
}

pub fn content_security_policy(config: &Config) -> String {
    let mut connect = vec!["'self'".to_owned()];
    // Gateway-WebSocket explizit (CSP 2 deckt ws(s) nicht über 'self' ab).
    connect.extend(ws_and_http(&config.public_url).into_iter().filter(|s| s.starts_with("ws")));
    if let Some(livekit) = &config.livekit {
        connect.extend(ws_and_http(&livekit.url));
    }
    connect.dedup();
    [
        "default-src 'self'".to_owned(),
        "script-src 'self'".to_owned(),
        "style-src 'self' 'unsafe-inline'".to_owned(),
        "img-src 'self' data: blob:".to_owned(),
        "media-src 'self' blob:".to_owned(),
        "font-src 'self'".to_owned(),
        format!("connect-src {}", connect.join(" ")),
        "worker-src 'self' blob:".to_owned(),
        "manifest-src 'self'".to_owned(),
        "object-src 'none'".to_owned(),
        "base-uri 'self'".to_owned(),
        "frame-ancestors 'none'".to_owned(),
        "form-action 'self'".to_owned(),
    ]
    .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LiveKitConfig;

    #[test]
    fn origins() {
        assert_eq!(
            ws_and_http("wss://rtc.example.org/"),
            ["wss://rtc.example.org", "https://rtc.example.org"]
        );
        assert_eq!(ws_and_http("http://localhost:7880"), ["ws://localhost:7880", "http://localhost:7880"]);
        assert_eq!(
            ws_and_http("https://a:b@chat.example.org/x?y"),
            ["wss://chat.example.org", "https://chat.example.org"]
        );
        assert!(ws_and_http("ftp://x").is_empty());
        assert!(ws_and_http("wss://bad host; script-src *").is_empty());
        assert!(ws_and_http("kein-url").is_empty());
    }

    #[test]
    fn policy_includes_livekit_and_gateway() {
        let mut config = Config::for_tests();
        config.public_url = "https://chat.example.org".into();
        config.livekit =
            Some(LiveKitConfig { url: "wss://rtc.example.org".into(), ..LiveKitConfig::for_tests() });
        let csp = content_security_policy(&config);
        assert!(csp.contains(
            "connect-src 'self' wss://chat.example.org wss://rtc.example.org https://rtc.example.org;"
        ));
        assert!(csp.contains("script-src 'self';"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }
}
