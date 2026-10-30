//! Interner Event-Bus für Echtzeit-Updates an die Gateway-Verbindungen.
//!
//! Aktuell ein In-Process-Broadcast. Für mehrere Server-Instanzen wird hier später ein
//! NATS-Bridge eingehängt (publish → NATS, NATS → lokaler Broadcast); Aufrufer bleiben gleich.

use std::sync::{Arc, OnceLock};

use axum::extract::ws::Utf8Bytes;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

/// An wen ein Event geht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Alle Mitglieder der Community; mit `channel_id` nur, wer den Kanal sehen darf.
    Community { community_id: Uuid, channel_id: Option<Uuid> },
    /// Alle Teilnehmer eines DM-Kanals.
    Dm(Uuid),
    /// Alle Verbindungen eines Nutzers.
    User(Uuid),
    /// Genau eine Session (z. B. Widerruf).
    Session(Uuid),
    /// Alle angemeldeten Verbindungen der Instanz (z. B. `INSTANCE_UPDATE`).
    Instance,
}

/// Ein Event, wie es an Clients geht: `{ "t": kind, "d": data }`.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    #[serde(rename = "t")]
    pub kind: &'static str,
    #[serde(rename = "d")]
    pub data: Value,
}

#[derive(Debug, Clone)]
pub struct BusEvent {
    pub scope: Scope,
    pub event: Event,
    /// Ändert sich durch das Event, wer welche Kanäle sehen darf? Dann verwerfen Verbindungen ihren Rechte-Cache.
    pub affects_permissions: bool,
    /// Interne Steuerung, wird nicht an Clients gesendet.
    pub control: Option<Control>,
    /// Weitere Empfänger zusätzlich zu `scope` (siehe `EventBus::publish_any`). Jede Verbindung
    /// bekommt das Event höchstens einmal, auch wenn mehrere Scopes auf sie passen.
    pub also: Vec<Scope>,
    /// Einmal serialisierter Text (`{"t":…,"d":…}`), geteilt von allen Verbindungen.
    text: OnceLock<Utf8Bytes>,
}

impl BusEvent {
    fn new(scope: Scope, event: Event, affects_permissions: bool, control: Option<Control>) -> Self {
        Self { scope, event, affects_permissions, control, also: Vec::new(), text: OnceLock::new() }
    }

    /// Das Event als WebSocket-Text. Wird beim ersten Aufruf serialisiert und dann geteilt – bei
    /// N Empfängern also einmal statt N-mal (`Utf8Bytes` klont nur einen Referenzzähler).
    pub fn text(&self) -> Utf8Bytes {
        self.text
            .get_or_init(|| match serde_json::to_string(&self.event) {
                Ok(text) => text.into(),
                Err(e) => {
                    tracing::error!(error = %e, "Event nicht serialisierbar");
                    Utf8Bytes::from_static("{}")
                }
            })
            .clone()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    /// Session wurde widerrufen → Verbindung schließen.
    SessionRevoked,
    /// Nutzer ist der Community beigetreten → in die Abo-Liste aufnehmen.
    Joined { user_id: Uuid, community_id: Uuid },
    /// Nutzer hat die Community verlassen → aus der Abo-Liste entfernen.
    Left { user_id: Uuid, community_id: Uuid },
    /// Nutzer nimmt (jetzt) am DM-Kanal teil → in die DM-Abo-Liste aufnehmen.
    DmJoined { user_id: Uuid, channel_id: Uuid },
    /// Nutzer nimmt nicht mehr am DM-Kanal teil → aus der DM-Abo-Liste entfernen.
    DmLeft { user_id: Uuid, channel_id: Uuid },
    /// Neue Freundschaft → der Nutzer verteilt seine Präsenz (auch) an `friend_id`.
    FriendAdded { user_id: Uuid, friend_id: Uuid },
    /// Freundschaft beendet (entfernt, blockiert, Konto gelöscht).
    FriendRemoved { user_id: Uuid, friend_id: Uuid },
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<BusEvent>>,
}

impl Default for EventBus {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self { tx }
    }
}

impl EventBus {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BusEvent>> {
        self.tx.subscribe()
    }

    fn send(&self, event: BusEvent) {
        // Kein Empfänger verbunden ist kein Fehler.
        let _ = self.tx.send(Arc::new(event));
    }

    pub fn publish(&self, scope: Scope, kind: &'static str, data: impl Serialize) {
        self.send(BusEvent::new(scope, Event { kind, data: to_value(data) }, false, None));
    }

    /// Ein Event an die Vereinigung mehrerer Scopes (ohne kanalgebundene Community-Scopes): Wer in
    /// mehreren davon ist, bekommt es trotzdem nur einmal. Ohne Scopes passiert nichts.
    pub fn publish_any(
        &self,
        scopes: impl IntoIterator<Item = Scope>,
        kind: &'static str,
        data: impl Serialize,
    ) {
        let mut scopes = scopes.into_iter();
        let Some(first) = scopes.next() else { return };
        let mut event = BusEvent::new(first, Event { kind, data: to_value(data) }, false, None);
        event.also = scopes.collect();
        self.send(event);
    }

    /// Wie `publish`, aber Verbindungen der Community berechnen ihre Kanal-Rechte neu.
    pub fn publish_permissions(&self, scope: Scope, kind: &'static str, data: impl Serialize) {
        self.send(BusEvent::new(scope, Event { kind, data: to_value(data) }, true, None));
    }

    pub fn control(&self, scope: Scope, control: Control) {
        self.send(BusEvent::new(scope, Event { kind: "", data: Value::Null }, true, Some(control)));
    }
}

fn to_value(data: impl Serialize) -> Value {
    serde_json::to_value(data).unwrap_or_else(|e| {
        tracing::error!(error = %e, "Event nicht serialisierbar");
        Value::Null
    })
}

pub fn community(community_id: Uuid) -> Scope {
    Scope::Community { community_id, channel_id: None }
}

/// Scope eines kanalgebundenen Events: in Community-Kanälen alle, die den Kanal sehen dürfen,
/// in DM-Kanälen (`community_id = None`) alle Teilnehmer.
pub fn channel(community_id: impl Into<Option<Uuid>>, channel_id: Uuid) -> Scope {
    match community_id.into() {
        Some(community_id) => Scope::Community { community_id, channel_id: Some(channel_id) },
        None => Scope::Dm(channel_id),
    }
}
