//! Flüchtige Präsenz (online/idle/dnd/offline) und Drosselung der Tipp-Anzeige.
//!
//! Bewusst **nicht** in Postgres: Präsenz ist ein Momentzustand, ein „zuletzt online“ wird nirgends
//! gespeichert. Ein Nutzer ist online, solange er mindestens eine Gateway-Verbindung hat; jede
//! Verbindung meldet ihren eigenen Status, daraus wird der sichtbare Status des Nutzers gebildet.
//!
//! Mehrere Server-Instanzen: Diese Registry lebt pro Prozess. Hinter einem Load Balancer muss sie
//! in einen geteilten Speicher mit TTL (z. B. NATS-KV oder Redis, Schlüssel pro Verbindung mit
//! Heartbeat) wandern; die Gateway-Aufrufe (`connect`/`set`/`disconnect`/`visible`) bleiben gleich.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status, den eine Verbindung meldet (`presence`-Op).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChosenStatus {
    Online,
    Idle,
    Dnd,
    Invisible,
}

/// Status, wie ihn andere sehen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Online,
    Idle,
    Dnd,
    Offline,
}

/// Mindestabstand zwischen zwei `TYPING_START` desselben Nutzers im selben Kanal.
pub const TYPING_THROTTLE: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Inner {
    /// user_id → (Verbindungs-ID → gemeldeter Status)
    users: HashMap<Uuid, HashMap<u64, ChosenStatus>>,
    /// (user_id, channel_id) → Zeitpunkt des letzten weitergegebenen Tipp-Events
    typing: HashMap<(Uuid, Uuid), Instant>,
}

#[derive(Clone, Default)]
pub struct Presence {
    inner: Arc<Mutex<Inner>>,
    next_conn: Arc<AtomicU64>,
}

/// Sichtbarer Status aus den Status aller Verbindungen eines Nutzers.
/// Bewusste Entscheidungen gewinnen: unsichtbar > bitte nicht stören > online > abwesend.
fn aggregate(conns: Option<&HashMap<u64, ChosenStatus>>) -> Status {
    let Some(conns) = conns.filter(|c| !c.is_empty()) else {
        return Status::Offline;
    };
    let has = |s| conns.values().any(|v| *v == s);
    if has(ChosenStatus::Invisible) {
        Status::Offline
    } else if has(ChosenStatus::Dnd) {
        Status::Dnd
    } else if has(ChosenStatus::Online) {
        Status::Online
    } else {
        Status::Idle
    }
}

impl Presence {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Ein Panic unter dem Lock lässt nur Präsenzdaten zurück – weiterarbeiten statt alles abzureißen.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Neue Verbindung registrieren. Liefert ihre ID und – falls sich der sichtbare Status
    /// geändert hat – den neuen Status.
    pub fn connect(&self, user_id: Uuid, status: ChosenStatus) -> (u64, Option<Status>) {
        let conn = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.lock();
        let before = aggregate(inner.users.get(&user_id));
        inner.users.entry(user_id).or_default().insert(conn, status);
        let after = aggregate(inner.users.get(&user_id));
        (conn, (before != after).then_some(after))
    }

    /// Status einer Verbindung ändern; liefert den neuen sichtbaren Status, falls geändert.
    pub fn set(&self, user_id: Uuid, conn: u64, status: ChosenStatus) -> Option<Status> {
        let mut inner = self.lock();
        let before = aggregate(inner.users.get(&user_id));
        inner.users.entry(user_id).or_default().insert(conn, status);
        let after = aggregate(inner.users.get(&user_id));
        (before != after).then_some(after)
    }

    /// Verbindung abmelden; nach der letzten Verbindung ist der Nutzer offline.
    pub fn disconnect(&self, user_id: Uuid, conn: u64) -> Option<Status> {
        let mut inner = self.lock();
        let before = aggregate(inner.users.get(&user_id));
        if let Some(conns) = inner.users.get_mut(&user_id) {
            conns.remove(&conn);
            if conns.is_empty() {
                inner.users.remove(&user_id);
            }
        }
        let after = aggregate(inner.users.get(&user_id));
        (before != after).then_some(after)
    }

    /// Sichtbarer Status eines Nutzers.
    pub fn visible(&self, user_id: Uuid) -> Status {
        aggregate(self.lock().users.get(&user_id))
    }

    /// Sichtbarer Status mehrerer Nutzer (für READY), nur wer nicht offline ist – fehlende sind offline.
    pub fn snapshot(&self, user_ids: &[Uuid]) -> Vec<(Uuid, Status)> {
        let inner = self.lock();
        user_ids
            .iter()
            .map(|id| (*id, aggregate(inner.users.get(id))))
            .filter(|(_, status)| *status != Status::Offline)
            .collect()
    }

    /// Anzahl der Nutzer, die für andere sichtbar online sind (online/abwesend/bitte nicht stören).
    /// Unsichtbare zählen bewusst nicht mit – auch nicht für Admins.
    pub fn online_count(&self) -> usize {
        self.lock().users.values().filter(|c| aggregate(Some(c)) != Status::Offline).count()
    }

    /// Darf für (Nutzer, Kanal) jetzt ein `TYPING_START` raus? Merkt sich den Zeitpunkt, falls ja.
    pub fn typing_allowed(&self, user_id: Uuid, channel_id: Uuid) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.typing.len() > 4096 {
            inner.typing.retain(|_, t| now.duration_since(*t) < TYPING_THROTTLE);
        }
        match inner.typing.get(&(user_id, channel_id)) {
            Some(last) if now.duration_since(*last) < TYPING_THROTTLE => false,
            _ => {
                inner.typing.insert((user_id, channel_id), now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_until_last_connection_closes() {
        let p = Presence::default();
        let u = Uuid::now_v7();
        let (a, change) = p.connect(u, ChosenStatus::Online);
        assert_eq!(change, Some(Status::Online));
        let (b, change) = p.connect(u, ChosenStatus::Online);
        assert_eq!(change, None);
        assert_eq!(p.disconnect(u, a), None);
        assert_eq!(p.visible(u), Status::Online);
        assert_eq!(p.disconnect(u, b), Some(Status::Offline));
        assert_eq!(p.visible(u), Status::Offline);
    }

    #[test]
    fn chosen_status_wins_over_idle() {
        let p = Presence::default();
        let u = Uuid::now_v7();
        let (a, _) = p.connect(u, ChosenStatus::Online);
        let (b, _) = p.connect(u, ChosenStatus::Online);
        // Ein Gerät abwesend, das andere aktiv → online.
        assert_eq!(p.set(u, a, ChosenStatus::Idle), None);
        // Beide abwesend → idle.
        assert_eq!(p.set(u, b, ChosenStatus::Idle), Some(Status::Idle));
        assert_eq!(p.set(u, a, ChosenStatus::Dnd), Some(Status::Dnd));
        assert_eq!(p.set(u, b, ChosenStatus::Invisible), Some(Status::Offline));
    }

    #[test]
    fn typing_is_throttled_per_user_and_channel() {
        let p = Presence::default();
        let (u, c) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(p.typing_allowed(u, c));
        assert!(!p.typing_allowed(u, c));
        assert!(p.typing_allowed(u, Uuid::now_v7()));
        assert!(p.typing_allowed(Uuid::now_v7(), c));
    }
}
