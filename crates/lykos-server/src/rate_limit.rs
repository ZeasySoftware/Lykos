//! Rate-Limiting für Anmeldung, Registrierung, Freundschaftsanfragen, DMs, Anrufe und Nachrichten.
//!
//! Bewusst nur im Arbeitsspeicher: Für den Pilot läuft genau eine Server-Instanz. Sobald mehrere
//! Instanzen hinter einem Load Balancer laufen, müssen die Zähler nach Redis (oder Postgres)
//! wandern, sonst vervielfacht sich das Limit mit der Zahl der Instanzen.
//!
//! Datenschutz: Client-IPs werden nur als Zählerschlüssel im Speicher gehalten, nie persistiert
//! oder geloggt, und verschwinden spätestens mit dem Ablauf ihres Zeitfensters.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Fehlgeschlagene Anmeldungen pro Benutzername (klein geschrieben) **und** Client-IP: Fremde können ein
/// Konto so nicht von ihrer IP aus für alle anderen aussperren; gegen verteiltes Raten wirkt zusätzlich
/// `LOGIN_PER_IP` je Quelle.
const LOGIN_PER_USER: Limit = Limit { max: 10, window: Duration::from_secs(15 * 60) };
/// Fehlgeschlagene Anmeldungen pro Client-IP.
const LOGIN_PER_IP: Limit = Limit { max: 30, window: Duration::from_secs(15 * 60) };
/// Registrierungsversuche pro Client-IP.
const REGISTER_PER_IP: Limit = Limit { max: 5, window: Duration::from_secs(60 * 60) };
/// „Passwort vergessen“-Anfragen pro Client-IP.
const RESET_PER_IP: Limit = Limit { max: 5, window: Duration::from_secs(60 * 60) };
/// Reset-Mails pro Konto (darüber hinaus wird still nichts verschickt).
const RESET_PER_ACCOUNT: Limit = Limit { max: 3, window: Duration::from_secs(60 * 60) };
/// Bestätigungsmails und E-Mail-Codes pro Konto.
const MAIL_PER_ACCOUNT: Limit = Limit { max: 5, window: Duration::from_secs(60 * 60) };
/// Freundschaftsanfragen, Annehmen, Blockieren und Entfernen pro Nutzer (Schutz vor Anfrage-Spam).
const RELATIONSHIPS_PER_USER: Limit = Limit { max: 30, window: Duration::from_secs(60) };
/// Neu eröffnete bzw. abgerufene DMs (`POST /dms`) pro Nutzer.
const DM_OPEN_PER_USER: Limit = Limit { max: 10, window: Duration::from_secs(60) };
/// Anklingeln in DMs (Anruf starten oder erneut klingeln lassen) pro Nutzer – Schutz vor Klingel-Spam.
const CALL_RING_PER_USER: Limit = Limit { max: 8, window: Duration::from_secs(60) };
/// Gesendete Nachrichten pro Nutzer über alle Kanäle – großzügig, kurze Schübe (Einfügen, schnelles
/// Hin und Her) bleiben möglich, Dauerfeuer nicht.
const MESSAGES_PER_USER: Limit = Limit { max: 120, window: Duration::from_secs(60) };

/// So oft werden abgelaufene Einträge mindestens entfernt.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
struct Limit {
    max: u32,
    window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    LoginUser(String, Option<IpAddr>),
    LoginIp(IpAddr),
    RegisterIp(IpAddr),
    ResetIp(IpAddr),
    ResetAccount(uuid::Uuid),
    MailAccount(uuid::Uuid),
    Relationships(uuid::Uuid),
    DmOpen(uuid::Uuid),
    CallRing(uuid::Uuid),
    Messages(uuid::Uuid),
}

impl Key {
    fn limit(&self) -> Limit {
        match self {
            Key::LoginUser(..) => LOGIN_PER_USER,
            Key::LoginIp(_) => LOGIN_PER_IP,
            Key::RegisterIp(_) => REGISTER_PER_IP,
            Key::ResetIp(_) => RESET_PER_IP,
            Key::ResetAccount(_) => RESET_PER_ACCOUNT,
            Key::MailAccount(_) => MAIL_PER_ACCOUNT,
            Key::Relationships(_) => RELATIONSHIPS_PER_USER,
            Key::DmOpen(_) => DM_OPEN_PER_USER,
            Key::CallRing(_) => CALL_RING_PER_USER,
            Key::Messages(_) => MESSAGES_PER_USER,
        }
    }
}

/// Festes Zeitfenster ab dem ersten gezählten Versuch.
#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, Window>,
    last_cleanup: Option<Instant>,
}

/// Zähler für Anmelde- und Registrierungsversuche. Billig klonbar (geteilter Zustand).
#[derive(Clone, Default)]
pub struct RateLimiter {
    inner: Arc<Mutex<Inner>>,
}

impl RateLimiter {
    /// Vor einer Anmeldung: `RateLimited`, wenn Benutzername+IP oder die IP gesperrt sind.
    pub fn check_login(&self, username: &str, ip: Option<IpAddr>) -> ApiResult<()> {
        self.check(&login_keys(username, ip))
    }

    /// Nach einer fehlgeschlagenen Anmeldung.
    pub fn login_failed(&self, username: &str, ip: Option<IpAddr>) {
        self.hit(&login_keys(username, ip));
    }

    /// Nach einer erfolgreichen Anmeldung: Fehlversuche des Benutzernamens von dieser IP vergessen.
    /// Der IP-Zähler bleibt, damit ein gültiges Konto nicht zum Zurücksetzen missbraucht wird.
    pub fn login_succeeded(&self, username: &str, ip: Option<IpAddr>) {
        let mut inner = self.lock();
        inner.entries.remove(&Key::LoginUser(normalize_username(username), ip));
    }

    /// Zählt einen Registrierungsversuch; `RateLimited`, wenn das Limit der IP erreicht ist.
    /// Ohne bekannte IP (z. B. in Tests ohne `ConnectInfo`) gibt es kein Limit.
    pub fn register_attempt(&self, ip: Option<IpAddr>) -> ApiResult<()> {
        let Some(ip) = ip else { return Ok(()) };
        let keys = [Key::RegisterIp(ip)];
        self.check(&keys)?;
        self.hit(&keys);
        Ok(())
    }

    /// Zählt eine „Passwort vergessen“-Anfrage der IP; `RateLimited`, wenn das Limit erreicht ist.
    /// Verrät nichts über Konten, da es nur an der IP hängt.
    pub fn reset_request(&self, ip: Option<IpAddr>) -> ApiResult<()> {
        let Some(ip) = ip else { return Ok(()) };
        self.attempt(&[Key::ResetIp(ip)])
    }

    /// Darf für dieses Konto noch eine Reset-Mail raus? Zählt mit. Bei `false` wird still nichts
    /// verschickt (die Antwort bleibt gleich, damit niemand Konten erkennt).
    pub fn reset_account(&self, user_id: uuid::Uuid) -> bool {
        self.attempt(&[Key::ResetAccount(user_id)]).is_ok()
    }

    /// Zählt eine Bestätigungsmail bzw. einen E-Mail-Code für das Konto.
    pub fn mail_to_account(&self, user_id: uuid::Uuid) -> ApiResult<()> {
        self.attempt(&[Key::MailAccount(user_id)])
    }

    /// Zählt eine Änderung an Beziehungen (Anfrage, Annehmen, Blockieren, Entfernen) des Nutzers.
    pub fn relationship_change(&self, user_id: uuid::Uuid) -> ApiResult<()> {
        self.attempt(&[Key::Relationships(user_id)])
    }

    /// Zählt ein `POST /dms` des Nutzers.
    pub fn dm_open(&self, user_id: uuid::Uuid) -> ApiResult<()> {
        self.attempt(&[Key::DmOpen(user_id)])
    }

    /// Zählt ein Anklingeln (Anruf in einer DM starten bzw. erneut klingeln lassen) des Nutzers.
    pub fn call_ring(&self, user_id: uuid::Uuid) -> ApiResult<()> {
        self.attempt(&[Key::CallRing(user_id)])
    }

    /// Zählt eine gesendete Nachricht des Nutzers.
    pub fn message_send(&self, user_id: uuid::Uuid) -> ApiResult<()> {
        self.attempt(&[Key::Messages(user_id)])
    }

    fn attempt(&self, keys: &[Key]) -> ApiResult<()> {
        self.check(keys)?;
        self.hit(keys);
        Ok(())
    }

    /// Entfernt alle abgelaufenen Einträge.
    pub fn cleanup(&self) {
        let mut inner = self.lock();
        cleanup(&mut inner, Instant::now());
    }

    /// Anzahl der Einträge (für Tests).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Ein Panic unter dem Lock lässt nur Zähler zurück – weiterarbeiten ist unkritisch.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check(&self, keys: &[Key]) -> ApiResult<()> {
        let now = Instant::now();
        let inner = self.lock();
        let retry_after = keys
            .iter()
            .filter_map(|key| {
                let w = inner.entries.get(key)?;
                let limit = key.limit();
                let elapsed = now.duration_since(w.started);
                (elapsed < limit.window && w.count >= limit.max).then(|| limit.window - elapsed)
            })
            .max();
        match retry_after {
            Some(wait) => Err(ApiError::RateLimited { retry_after: wait.as_secs().max(1) }),
            None => Ok(()),
        }
    }

    fn hit(&self, keys: &[Key]) {
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.last_cleanup.is_none_or(|t| now.duration_since(t) >= CLEANUP_INTERVAL) {
            cleanup(&mut inner, now);
        }
        for key in keys {
            let window = key.limit().window;
            let w = inner.entries.entry(key.clone()).or_insert(Window { started: now, count: 0 });
            if now.duration_since(w.started) >= window {
                *w = Window { started: now, count: 0 };
            }
            w.count = w.count.saturating_add(1);
        }
    }
}

fn cleanup(inner: &mut Inner, now: Instant) {
    inner.entries.retain(|key, w| now.duration_since(w.started) < key.limit().window);
    inner.last_cleanup = Some(now);
}

/// Zählerschlüssel einer Anmeldung: Benutzername+IP und – falls bekannt – die IP allein.
fn login_keys(username: &str, ip: Option<IpAddr>) -> Vec<Key> {
    let mut keys = vec![Key::LoginUser(normalize_username(username), ip)];
    keys.extend(ip.map(Key::LoginIp));
    keys
}

/// Benutzername als Zählerschlüssel: getrimmt, klein, begrenzt (schützt den Speicher vor Riesen-Keys).
fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase().chars().take(64).collect()
}

/// IPv6-Adressen werden auf ihr /64-Präfix gekürzt – ein Anschluss hat typischerweise ein
/// ganzes /64 und könnte sonst beliebig viele Adressen durchprobieren.
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

/// Räumt im Hintergrund regelmäßig abgelaufene Einträge auf, damit IPs nicht länger als nötig
/// im Speicher bleiben – auch wenn gerade niemand Anmeldeversuche macht.
pub fn spawn_cleanup(limiter: RateLimiter) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            tick.tick().await;
            limiter.cleanup();
        }
    });
}

/// Client-IP für das Rate-Limiting.
///
/// Stammt die Verbindung von einem vertrauenswürdigen Reverse Proxy (`LYKOS_TRUSTED_PROXIES`, ersatzweise
/// jede Verbindung bei `LYKOS_TRUST_PROXY=true`), gilt `X-Forwarded-For`, sonst die Socket-Adresse
/// (`ConnectInfo`). `None`, wenn keine bekannt ist (z. B. in Tests ohne `ConnectInfo`) – dann
/// greifen nur die Limits pro Benutzername.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let socket = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(addr)| addr.ip());
        let forwarded =
            from_trusted_proxy(&state.config, socket).then(|| forwarded_ip(&parts.headers)).flatten();
        Ok(Self(forwarded.or(socket).map(normalize_ip)))
    }
}

/// Darf diese Verbindung `X-Forwarded-For` setzen? Mit `LYKOS_TRUSTED_PROXIES` nur die genannten
/// Proxys – sonst könnte jeder, der den Port direkt erreicht, seine IP fälschen und die IP-Limits umgehen.
fn from_trusted_proxy(config: &crate::config::Config, socket: Option<IpAddr>) -> bool {
    if config.trusted_proxies.is_empty() {
        return config.trust_proxy;
    }
    socket.is_some_and(|ip| config.trusted_proxies.iter().any(|net| net.contains(ip)))
}

/// Client-Adresse aus `X-Forwarded-For`.
///
/// Genommen wird der **letzte** Eintrag: Den hängt der vertrauenswürdige Reverse Proxy
/// (Nginx Proxy Manager, `$proxy_add_x_forwarded_for`) selbst an. Alle Einträge davor kommen
/// vom Client und sind beliebig fälschbar. Gilt für genau einen Proxy-Hop vor Lykos.
fn forwarded_ip(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    let value = headers.get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).next_back()?;
    value.rsplit(',').map(str::trim).find(|s| !s.is_empty())?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn login_locks_after_max_failures_per_user() {
        let rl = RateLimiter::default();
        for _ in 0..LOGIN_PER_USER.max {
            assert!(rl.check_login("Alex", None).is_ok());
            rl.login_failed("Alex", None);
        }
        let err = rl.check_login(" alex ", None).unwrap_err();
        assert!(
            matches!(err, ApiError::RateLimited { retry_after } if retry_after > 0 && retry_after <= 900)
        );
        // Anderer Name ist nicht betroffen.
        assert!(rl.check_login("bob", None).is_ok());
    }

    #[test]
    fn success_resets_user_counter() {
        let rl = RateLimiter::default();
        for _ in 0..LOGIN_PER_USER.max - 1 {
            rl.login_failed("alex", None);
        }
        rl.login_succeeded("ALEX", None);
        rl.login_failed("alex", None);
        assert!(rl.check_login("alex", None).is_ok());
    }

    #[test]
    fn user_lock_is_per_ip() {
        let rl = RateLimiter::default();
        let attacker: IpAddr = "10.0.0.66".parse().unwrap();
        let owner: IpAddr = "10.0.0.7".parse().unwrap();
        for _ in 0..LOGIN_PER_USER.max {
            rl.login_failed("alex", Some(attacker));
        }
        // Von der IP des Angreifers gesperrt, der Kontoinhaber kommt weiter rein.
        assert!(rl.check_login("alex", Some(attacker)).is_err());
        assert!(rl.check_login("alex", Some(owner)).is_ok());
        // Erfolg auf einer IP setzt nur deren Zähler zurück.
        rl.login_succeeded("alex", Some(owner));
        assert!(rl.check_login("alex", Some(attacker)).is_err());
    }

    #[test]
    fn ip_limit_spans_usernames() {
        let rl = RateLimiter::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for i in 0..LOGIN_PER_IP.max {
            rl.login_failed(&format!("user{i}"), Some(ip));
        }
        assert!(rl.check_login("neu", Some(ip)).is_err());
        assert!(rl.check_login("neu", Some("10.0.0.2".parse().unwrap())).is_ok());
    }

    #[test]
    fn register_limit_per_ip() {
        let rl = RateLimiter::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..REGISTER_PER_IP.max {
            assert!(rl.register_attempt(Some(ip)).is_ok());
        }
        assert!(rl.register_attempt(Some(ip)).is_err());
        assert!(rl.register_attempt(None).is_ok());
    }

    #[test]
    fn reset_and_mail_limits() {
        let rl = RateLimiter::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..RESET_PER_IP.max {
            assert!(rl.reset_request(Some(ip)).is_ok());
        }
        assert!(rl.reset_request(Some(ip)).is_err());
        let user = uuid::Uuid::now_v7();
        for _ in 0..RESET_PER_ACCOUNT.max {
            assert!(rl.reset_account(user));
        }
        assert!(!rl.reset_account(user));
        assert!(rl.reset_account(uuid::Uuid::now_v7()));
        for _ in 0..MAIL_PER_ACCOUNT.max {
            assert!(rl.mail_to_account(user).is_ok());
        }
        assert!(rl.mail_to_account(user).is_err());
    }

    #[test]
    fn per_user_action_limits() {
        let rl = RateLimiter::default();
        let user = uuid::Uuid::now_v7();
        for _ in 0..RELATIONSHIPS_PER_USER.max {
            assert!(rl.relationship_change(user).is_ok());
        }
        assert!(rl.relationship_change(user).is_err());
        for _ in 0..DM_OPEN_PER_USER.max {
            assert!(rl.dm_open(user).is_ok());
        }
        assert!(rl.dm_open(user).is_err());
        for _ in 0..MESSAGES_PER_USER.max {
            assert!(rl.message_send(user).is_ok());
        }
        assert!(
            matches!(rl.message_send(user), Err(ApiError::RateLimited { retry_after }) if retry_after <= 60)
        );
        // Andere Nutzer sind nicht betroffen.
        let other = uuid::Uuid::now_v7();
        assert!(
            rl.relationship_change(other).is_ok()
                && rl.dm_open(other).is_ok()
                && rl.message_send(other).is_ok()
        );
    }

    #[test]
    fn forwarded_header_only_from_trusted_proxies() {
        let mut config = crate::config::Config::for_tests();
        let proxy: IpAddr = "10.0.0.5".parse().unwrap();
        let direct: IpAddr = "198.51.100.9".parse().unwrap();

        // Standard: niemandem trauen.
        assert!(!from_trusted_proxy(&config, Some(proxy)));
        // Alt: LYKOS_TRUST_PROXY=true ohne Liste traut jeder Verbindung.
        config.trust_proxy = true;
        assert!(from_trusted_proxy(&config, Some(direct)));
        // Mit Liste nur den genannten Proxys – auch wenn LYKOS_TRUST_PROXY gesetzt ist.
        config.trusted_proxies = vec!["10.0.0.5".parse().unwrap()];
        assert!(from_trusted_proxy(&config, Some(proxy)));
        assert!(from_trusted_proxy(&config, Some("::ffff:10.0.0.5".parse().unwrap())));
        assert!(!from_trusted_proxy(&config, Some(direct)));
        assert!(!from_trusted_proxy(&config, None));
    }

    #[test]
    fn expired_entries_are_removed() {
        let rl = RateLimiter::default();
        rl.login_failed("alex", Some("10.0.0.1".parse().unwrap()));
        assert_eq!(rl.len(), 2);
        {
            let mut inner = rl.lock();
            let past = Instant::now() - Duration::from_secs(16 * 60);
            for w in inner.entries.values_mut() {
                w.started = past;
            }
        }
        assert!(rl.check_login("alex", None).is_ok());
        rl.cleanup();
        assert!(rl.is_empty());
    }

    #[test]
    fn ipv6_is_grouped_by_prefix() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        assert_eq!(normalize_ip(a), normalize_ip(b));
        let mapped: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(normalize_ip(mapped), "192.0.2.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn forwarded_for_uses_proxy_appended_entry() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "6.6.6.6, 203.0.113.7".parse().unwrap());
        assert_eq!(forwarded_ip(&h), Some("203.0.113.7".parse().unwrap()));
        h.insert("x-forwarded-for", "kaputt".parse().unwrap());
        assert_eq!(forwarded_ip(&h), None);
    }
}
