//! Reine Prüf-Bausteine des AutoMod (ohne Datenbank): Wortfilter, Link-Erkennung, Erwähnungen zählen und
//! der Flut-Zähler. Alles hier ist synchron und per Unit-Test abgedeckt.

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    time::{Duration, Instant},
};

use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::mentions;

/// Text für Vergleiche vereinheitlichen: NFKC (z. B. Vollbreite-Zeichen, Ligaturen → Grundform),
/// kleingeschrieben, Leerraum-Folgen zu einem Leerzeichen, ohne Rand-Leerraum.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.nfkc().flat_map(char::to_lowercase) {
        if c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
}

/// Buchstabe oder Ziffer – Wortgrenzen liegen zwischen Wort- und Nicht-Wortzeichen.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

// --- Wortfilter --------------------------------------------------------------------------------

/// Höchstzahl der Begriffe (und der Ausnahmen) je Wortfilter.
pub const MAX_TERMS: usize = 1000;
/// Höchstlänge eines Begriffs in Zeichen.
pub const MAX_TERM_CHARS: usize = 60;

#[derive(Debug, Clone)]
struct Term {
    /// Wie eingegeben (für den Alarm im Protokollkanal).
    original: String,
    /// Normalisierter Kern ohne Platzhalter.
    core: String,
    /// `*foo`: darf mitten in einem Wort beginnen (Wortende-/Enthält-Treffer).
    open_start: bool,
    /// `foo*`: darf mitten in einem Wort enden (Wortanfang-/Enthält-Treffer).
    open_end: bool,
}

/// Begriff prüfen und normalisieren; `None` bei ungültiger Eingabe (leer, zu lang, nur `*`,
/// `*` mitten im Begriff). Gibt den bereinigten Begriff zurück (getrimmt, Leerraum vereinheitlicht).
pub fn clean_term(raw: &str) -> Option<String> {
    let trimmed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() || trimmed.chars().count() > MAX_TERM_CHARS {
        return None;
    }
    let core = trimmed.trim_start_matches('*').trim_end_matches('*');
    // Platzhalter nur am Rand, höchstens je einer.
    let stars_start = trimmed.len() - trimmed.trim_start_matches('*').len();
    let stars_end = trimmed.len() - trimmed.trim_end_matches('*').len();
    if core.trim().is_empty() || core.contains('*') || stars_start > 1 || stars_end > 1 {
        return None;
    }
    Some(trimmed)
}

/// Wortfilter: Begriffe mit Platzhalter `*` am Anfang und/oder Ende, sonst ganze Wörter (bzw. Wortfolgen).
/// Groß-/Kleinschreibung egal, Unicode-normalisiert. Ausnahmen sind ganze Wörter, die nie als Treffer zählen
/// (z. B. „Klasse“ bei `*lass*`).
#[derive(Debug, Clone, Default)]
pub struct KeywordFilter {
    terms: Vec<Term>,
    allow: HashSet<String>,
}

impl KeywordFilter {
    /// Ungültige Einträge werden übergangen (die Verwaltung prüft vorher mit `clean_term`).
    pub fn new(terms: &[String], allow: &[String]) -> Self {
        let terms = terms
            .iter()
            .filter_map(|raw| {
                let cleaned = clean_term(raw)?;
                let open_start = cleaned.starts_with('*');
                let open_end = cleaned.ends_with('*');
                let core = normalize(cleaned.trim_matches('*'));
                (!core.is_empty()).then_some(Term { original: cleaned, core, open_start, open_end })
            })
            .collect();
        let allow = allow.iter().map(|a| normalize(a)).filter(|a| !a.is_empty()).collect();
        Self { terms, allow }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Erster getroffener Begriff (wie eingegeben) oder `None`.
    pub fn find(&self, content: &str) -> Option<&str> {
        if self.terms.is_empty() {
            return None;
        }
        let text = normalize(content);
        self.terms.iter().find(|t| self.term_matches(&text, t)).map(|t| t.original.as_str())
    }

    fn term_matches(&self, text: &str, term: &Term) -> bool {
        text.match_indices(term.core.as_str()).any(|(start, m)| {
            let end = start + m.len();
            let before = text[..start].chars().next_back();
            let after = text[end..].chars().next();
            let starts_ok = term.open_start || !before.is_some_and(is_word_char);
            let ends_ok = term.open_end || !after.is_some_and(is_word_char);
            starts_ok && ends_ok && !self.allowed(text, start, end)
        })
    }

    /// Liegt der Treffer in einem Wort, das auf der Ausnahmeliste steht?
    fn allowed(&self, text: &str, start: usize, end: usize) -> bool {
        if self.allow.is_empty() {
            return false;
        }
        let word_start = text[..start]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word_char(*c))
            .last()
            .map_or(start, |(i, _)| i);
        let word_end =
            text[end..].char_indices().find(|(_, c)| !is_word_char(*c)).map_or(text.len(), |(i, _)| end + i);
        self.allow.contains(&text[word_start..word_end])
    }
}

// --- Links ------------------------------------------------------------------------------------

/// Höchstzahl der Domains auf der Erlaubnisliste.
pub const MAX_DOMAINS: usize = 200;

/// Ein gefundener Link: Host (klein, ohne Port/Zugangsdaten) und ob es ein Einladungslink ist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoundLink {
    pub host: String,
    pub invite: bool,
}

/// Zeichen, die eine URL beenden (neben Leerraum).
fn ends_url(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '|' | '(' | ')' | '[' | ']' | '{' | '}')
}

/// Findet Links der Form `schema://host…` und `www.host…` (Groß-/Kleinschreibung egal).
pub fn find_links(content: &str) -> Vec<FoundLink> {
    let text: String = content.nfkc().flat_map(char::to_lowercase).collect();
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |rest: &str| {
        let url: &str = rest.find(ends_url).map_or(rest, |i| &rest[..i]);
        let host_end = url.find(['/', '?', '#', '\\']).unwrap_or(url.len());
        let (authority, path) = url.split_at(host_end);
        let host = authority.rsplit('@').next().unwrap_or("");
        let host = host.split(':').next().unwrap_or("");
        let host = host.trim_end_matches(['.', ',', ';', '!']).to_owned();
        if host.is_empty() || !host.chars().any(char::is_alphanumeric) {
            return;
        }
        let invite = path.starts_with("/invite/")
            || host == "discord.gg"
            || ((host == "discord.com" || host == "discordapp.com") && path.starts_with("/invite"));
        let link = FoundLink { host, invite };
        if seen.insert(link.clone()) {
            found.push(link);
        }
    };

    // `schema://`
    for (i, _) in text.match_indices("://") {
        let scheme = text[..i]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
            .last()
            .map(|(s, _)| &text[s..i]);
        if scheme.is_some_and(|s| s.starts_with(|c: char| c.is_ascii_alphabetic())) {
            push(&text[i + 3..]);
        }
    }
    // `www.` ohne Schema (nicht schon Teil von `…://www.`)
    for (i, _) in text.match_indices("www.") {
        let before = text[..i].chars().next_back();
        if !before.is_some_and(|c| is_word_char(c) || matches!(c, '/' | '.' | '-' | '@')) {
            push(&text[i..]);
        }
    }
    found
}

/// Domain für die Erlaubnisliste bereinigen: klein, ohne Schema, Pfad, Port und führendes `*.`/`.`.
/// `None`, wenn keine gültige Domain übrig bleibt.
pub fn clean_domain(raw: &str) -> Option<String> {
    let d = raw.trim().nfkc().flat_map(char::to_lowercase).collect::<String>();
    let d = d.split_once("://").map_or(d.as_str(), |(_, rest)| rest);
    let d = d.split(['/', '?', '#']).next().unwrap_or("");
    let d = d.rsplit('@').next().unwrap_or("");
    let d = d.split(':').next().unwrap_or("");
    let d = d.trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.');
    let valid = !d.is_empty()
        && d.len() <= 253
        && d.split('.').all(|label| !label.is_empty() && label.len() <= 63)
        && d.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.');
    valid.then(|| d.to_owned())
}

/// Gehört `host` zur Domain `domain` (gleich oder Subdomain)?
pub fn host_in_domain(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
}

/// Erlaubnisliste für Links. Links zur eigenen Instanz sind immer erlaubt.
#[derive(Debug, Clone, Default)]
pub struct LinkPolicy {
    pub allowed_domains: Vec<String>,
    /// Einladungslinks anderer Instanzen/Dienste auch dann blockieren, wenn die Domain erlaubt ist.
    pub block_foreign_invites: bool,
    /// Host der eigenen Instanz (aus `public_url`).
    pub own_host: Option<String>,
}

impl LinkPolicy {
    /// Erster nicht erlaubter Link oder `None`.
    pub fn violation(&self, content: &str) -> Option<FoundLink> {
        find_links(content).into_iter().find(|link| {
            if self.own_host.as_deref().is_some_and(|own| host_in_domain(&link.host, own)) {
                return false;
            }
            if link.invite && self.block_foreign_invites {
                return true;
            }
            !self.allowed_domains.iter().any(|d| host_in_domain(&link.host, d))
        })
    }
}

/// Host einer URL wie `https://chat.example.org:8443/pfad` → `chat.example.org`.
pub fn url_host(url: &str) -> Option<String> {
    find_links(url).into_iter().next().map(|l| l.host)
}

// --- Erwähnungen ------------------------------------------------------------------------------

/// Kandidaten für Nutzer-Erwähnungen (`@name`, ohne `@everyone` und Code): Anzahl verschiedener Namen
/// (`@bob.` zählt einmal) und alle Schreibweisen zum Abgleich mit der Datenbank.
pub fn mention_candidates(content: &str) -> (usize, Vec<String>) {
    let parsed = mentions::parse(content);
    let distinct: BTreeSet<&str> = parsed.usernames.iter().map(|n| n.trim_end_matches(['.', '-'])).collect();
    (distinct.len(), parsed.usernames)
}

// --- Flut ------------------------------------------------------------------------------------

/// Identischer Inhalt zählt nur als Wiederholung, wenn die letzte Nachricht höchstens so lange her ist.
const DUPLICATE_RESET: Duration = Duration::from_secs(10 * 60);
/// So lange bleibt ein Zähler ohne neue Nachricht im Speicher.
const FLOOD_IDLE: Duration = Duration::from_secs(10 * 60);
/// So oft werden verwaiste Zähler mindestens entfernt.
const FLOOD_CLEANUP: Duration = Duration::from_secs(60);

/// Grenzen der Spam-Regel; `0` schaltet den jeweiligen Teil ab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloodLimits {
    /// Mehr als so viele Nachrichten …
    pub max_messages: u32,
    /// … in so vielen Sekunden.
    pub per_seconds: u32,
    /// Mehr als so viele identische Nachrichten hintereinander.
    pub max_duplicates: u32,
}

/// Verstoß gegen die Spam-Regel. `first`: erster Verstoß dieser Serie (nur dann Alarm/Audit, damit
/// eine Flut nicht den Protokollkanal flutet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloodHit {
    /// `count` Nachrichten im Zeitfenster.
    Rate { count: u32, first: bool },
    /// `count`-mal derselbe Inhalt hintereinander.
    Duplicate { count: u32, first: bool },
}

impl FloodHit {
    pub fn first(self) -> bool {
        match self {
            FloodHit::Rate { first, .. } | FloodHit::Duplicate { first, .. } => first,
        }
    }
}

#[derive(Debug)]
struct FloodEntry {
    /// Zeitpunkte der letzten Nachrichten (höchstens `max_messages + 1`).
    times: VecDeque<Instant>,
    last_hash: Option<u64>,
    duplicate_run: u32,
    last_at: Instant,
}

/// In-Memory-Zähler je (Community, Autor) wie in `rate_limit.rs` – pro Serverprozess.
#[derive(Debug, Default)]
pub struct FloodTracker {
    entries: HashMap<(Uuid, Uuid), FloodEntry>,
    last_cleanup: Option<Instant>,
}

/// Inhalt für den Wiederholungsvergleich: normalisiert, leer = nicht vergleichen (nur Anhänge).
pub fn content_hash(content: &str) -> Option<u64> {
    let text = normalize(content);
    if text.is_empty() {
        return None;
    }
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    Some(h.finish())
}

impl FloodTracker {
    /// Zählt eine Nachricht von `author` in `community` und meldet einen Verstoß. Blockierte Versuche
    /// zählen mit – wer weiter flutet, bleibt gesperrt, bis er eine Weile pausiert.
    pub fn hit(
        &mut self,
        community: Uuid,
        author: Uuid,
        now: Instant,
        hash: Option<u64>,
        limits: FloodLimits,
    ) -> Option<FloodHit> {
        if self.last_cleanup.is_none_or(|t| now.duration_since(t) >= FLOOD_CLEANUP) {
            self.entries.retain(|_, e| now.duration_since(e.last_at) < FLOOD_IDLE);
            self.last_cleanup = Some(now);
        }
        let entry = self.entries.entry((community, author)).or_insert_with(|| FloodEntry {
            times: VecDeque::new(),
            last_hash: None,
            duplicate_run: 0,
            last_at: now,
        });

        let mut result = None;
        if limits.max_messages > 0 && limits.per_seconds > 0 {
            let window = Duration::from_secs(u64::from(limits.per_seconds));
            while entry.times.front().is_some_and(|t| now.duration_since(*t) >= window) {
                entry.times.pop_front();
            }
            // War der Puffer schon vor dieser Nachricht voll (alle im Fenster), war auch die vorige ein
            // Verstoß – dann ist das hier nicht der erste der Serie.
            let before = entry.times.len();
            entry.times.push_back(now);
            let cap = limits.max_messages as usize + 1;
            while entry.times.len() > cap {
                entry.times.pop_front();
            }
            // Der Puffer hält höchstens `max + 1` Zeitpunkte – reicht für die Entscheidung.
            let count = entry.times.len() as u32;
            if count > limits.max_messages {
                result = Some(FloodHit::Rate { count, first: before <= limits.max_messages as usize });
            }
        }
        if limits.max_duplicates > 0
            && let Some(hash) = hash
        {
            let repeat = entry.last_hash == Some(hash) && now.duration_since(entry.last_at) < DUPLICATE_RESET;
            entry.duplicate_run = if repeat { entry.duplicate_run.saturating_add(1) } else { 1 };
            if result.is_none() && entry.duplicate_run > limits.max_duplicates {
                let count = entry.duplicate_run;
                result = Some(FloodHit::Duplicate { count, first: count == limits.max_duplicates + 1 });
            }
        }
        if hash.is_some() {
            entry.last_hash = hash;
        }
        entry.last_at = now;
        result
    }

    /// Anzahl der Zähler (für Tests).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(terms: &[&str], allow: &[&str]) -> KeywordFilter {
        let terms: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
        let allow: Vec<String> = allow.iter().map(|s| s.to_string()).collect();
        KeywordFilter::new(&terms, &allow)
    }

    #[test]
    fn normalizes_unicode_case_and_space() {
        assert_eq!(normalize("  ＨＡＬＬＯ \t Welt\n"), "hallo welt");
        assert_eq!(normalize("Straße"), "straße");
        assert_eq!(normalize("ﬁsch"), "fisch");
    }

    #[test]
    fn keyword_whole_word_by_default() {
        let f = filter(&["spam"], &[]);
        assert_eq!(f.find("Das ist SPAM!"), Some("spam"));
        assert_eq!(f.find("spam"), Some("spam"));
        assert_eq!(f.find("spammer hier"), None);
        assert_eq!(f.find("antispam"), None);
    }

    #[test]
    fn keyword_wildcards() {
        let prefix = filter(&["spam*"], &[]);
        assert!(prefix.find("Spammer!").is_some());
        assert!(prefix.find("antispam").is_none());

        let suffix = filter(&["*spam"], &[]);
        assert!(suffix.find("antispam").is_some());
        assert!(suffix.find("spammer").is_none());

        let contains = filter(&["*spam*"], &[]);
        assert!(contains.find("xxspamxx").is_some());
        assert!(contains.find("sp am").is_none());
    }

    #[test]
    fn keyword_unicode_and_phrases() {
        let f = filter(&["böse wort"], &[]);
        assert!(f.find("Das BÖSE   Wort steht da").is_some());
        assert!(f.find("böseswort").is_none());
        // Vollbreite-Buchstaben werden per NFKC zu normalen.
        assert!(filter(&["spam"], &[]).find("ｓｐａｍ").is_some());
        // Großschreibung im Begriff egal.
        assert!(filter(&["ÄRGER"], &[]).find("so ein ärger").is_some());
    }

    #[test]
    fn keyword_allow_list() {
        let f = filter(&["*lass*"], &["klasse", "Lassen"]);
        assert!(f.find("Das ist Klasse").is_none());
        assert!(f.find("lassen wir das").is_none());
        assert_eq!(f.find("Das ist klasse, aber Blasse nicht"), Some("*lass*"));
    }

    #[test]
    fn clean_term_validates() {
        assert_eq!(clean_term("  foo   bar "), Some("foo bar".into()));
        assert_eq!(clean_term("*foo*"), Some("*foo*".into()));
        assert_eq!(clean_term("*"), None);
        assert_eq!(clean_term("**foo"), None);
        assert_eq!(clean_term("fo*o"), None);
        assert_eq!(clean_term(""), None);
        assert_eq!(clean_term(&"x".repeat(61)), None);
        assert!(clean_term(&"x".repeat(60)).is_some());
    }

    #[test]
    fn finds_links() {
        let links = find_links(
            "Schau https://Example.org/pfad?x=1 und www.test.de, mail@foo.de, http://user:pw@host.io:8080/a",
        );
        let hosts: Vec<&str> = links.iter().map(|l| l.host.as_str()).collect();
        assert_eq!(hosts, vec!["example.org", "host.io", "www.test.de"]);
        assert!(find_links("kein link, z.B. so. Oder foo://").is_empty());
        assert_eq!(find_links("[Text](https://a.b/c)")[0].host, "a.b");
        assert_eq!(find_links("<https://a.b>")[0].host, "a.b");
    }

    #[test]
    fn detects_invites() {
        assert!(find_links("https://chat.other.org/invite/abc")[0].invite);
        assert!(find_links("https://discord.gg/abc")[0].invite);
        assert!(!find_links("https://chat.other.org/channels")[0].invite);
    }

    #[test]
    fn link_policy_allow_list_with_subdomains() {
        let policy = LinkPolicy {
            allowed_domains: vec!["example.org".into()],
            block_foreign_invites: true,
            own_host: Some("lykos.local".into()),
        };
        assert!(policy.violation("https://example.org").is_none());
        assert!(policy.violation("https://docs.example.org/x").is_none());
        assert_eq!(policy.violation("https://badexample.org").unwrap().host, "badexample.org");
        assert_eq!(policy.violation("www.evil.com").unwrap().host, "www.evil.com");
        // Eigene Instanz immer erlaubt, auch Einladungen.
        assert!(policy.violation("https://lykos.local/invite/abc").is_none());
        // Fremde Einladung trotz erlaubter Domain.
        assert!(policy.violation("https://example.org/invite/abc").is_some());
        let lenient = LinkPolicy { block_foreign_invites: false, ..policy };
        assert!(lenient.violation("https://example.org/invite/abc").is_none());
        assert!(lenient.violation("ganz ohne Link").is_none());
    }

    #[test]
    fn cleans_domains() {
        assert_eq!(clean_domain(" HTTPS://Www.Example.org/pfad "), Some("www.example.org".into()));
        assert_eq!(clean_domain("*.example.org"), Some("example.org".into()));
        assert_eq!(clean_domain("example.org:8080"), Some("example.org".into()));
        assert_eq!(clean_domain("bad domain"), None);
        assert_eq!(clean_domain("a..b"), None);
        assert_eq!(clean_domain(""), None);
        assert!(host_in_domain("a.b.example.org", "example.org"));
        assert!(!host_in_domain("notexample.org", "example.org"));
    }

    #[test]
    fn counts_mentions() {
        assert_eq!(mention_candidates("@a1 @b2 @a1 @c3.").0, 3);
        assert_eq!(mention_candidates("@everyone `@x1` mail@y.de").0, 0);
    }

    fn limits(max_messages: u32, per_seconds: u32, max_duplicates: u32) -> FloodLimits {
        FloodLimits { max_messages, per_seconds, max_duplicates }
    }

    #[test]
    fn flood_rate_window() {
        let mut t = FloodTracker::default();
        let (c, u) = (Uuid::now_v7(), Uuid::now_v7());
        let start = Instant::now();
        let l = limits(3, 5, 0);
        for i in 0..3 {
            assert_eq!(t.hit(c, u, start + Duration::from_millis(i * 10), Some(i), l), None);
        }
        let hit = t.hit(c, u, start + Duration::from_millis(40), Some(9), l);
        assert_eq!(hit, Some(FloodHit::Rate { count: 4, first: true }));
        // Weiter fluten bleibt ein Verstoß, aber nicht mehr der erste.
        let again = t.hit(c, u, start + Duration::from_millis(50), Some(10), l).unwrap();
        assert!(!again.first());
        // Nach Ablauf des Fensters wieder frei.
        assert_eq!(t.hit(c, u, start + Duration::from_secs(6), Some(11), l), None);
        // Andere Nutzer und Communities zählen getrennt.
        assert_eq!(t.hit(Uuid::now_v7(), u, start, Some(1), l), None);
    }

    #[test]
    fn flood_duplicates() {
        let mut t = FloodTracker::default();
        let (c, u) = (Uuid::now_v7(), Uuid::now_v7());
        let now = Instant::now();
        let l = limits(0, 0, 2);
        let h = content_hash("Hallo");
        assert_eq!(h, content_hash("  HALLO "));
        assert_eq!(t.hit(c, u, now, h, l), None);
        assert_eq!(t.hit(c, u, now, h, l), None);
        assert_eq!(t.hit(c, u, now, h, l), Some(FloodHit::Duplicate { count: 3, first: true }));
        assert_eq!(t.hit(c, u, now, h, l), Some(FloodHit::Duplicate { count: 4, first: false }));
        // Anderer Inhalt setzt zurück; reine Anhänge (`None`) zählen nicht.
        assert_eq!(t.hit(c, u, now, content_hash("anders"), l), None);
        assert_eq!(content_hash("   "), None);
        // Nach langer Pause ist Wiederholung erlaubt.
        let later = now + DUPLICATE_RESET;
        assert_eq!(t.hit(c, u, later, content_hash("anders"), l), None);
    }

    #[test]
    fn flood_cleanup_drops_idle_entries() {
        let mut t = FloodTracker::default();
        let now = Instant::now();
        let l = limits(5, 5, 0);
        t.hit(Uuid::now_v7(), Uuid::now_v7(), now, None, l);
        assert_eq!(t.len(), 1);
        t.hit(Uuid::now_v7(), Uuid::now_v7(), now + FLOOD_IDLE + FLOOD_CLEANUP, None, l);
        assert_eq!(t.len(), 1);
    }
}
