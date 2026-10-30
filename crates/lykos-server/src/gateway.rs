//! WebSocket-Gateway: verteilt Events aus dem Bus an angemeldete Clients.
//! Protokoll siehe `docs/api.md`.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket},
    },
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::{Receiver, error::RecvError};
use uuid::Uuid;

use crate::{
    auth,
    error::ApiError,
    events::{self, BusEvent, Control, Scope},
    models,
    permissions::{self, Permissions},
    presence::{ChosenStatus, Status},
    routes::{calls, categories, relationships},
    state::AppState,
};

/// Close-Code für ungültige oder widerrufene Sessions.
const CLOSE_UNAUTHORIZED: u16 = 4001;
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
pub struct GatewayQuery {
    /// Anfangsstatus der Verbindung (`online`, `idle`, `dnd`, `invisible`), damit z. B. ein
    /// unsichtbarer Nutzer beim Verbinden nicht kurz als online erscheint.
    status: Option<String>,
}

/// Origin ohne Pfad und abschließenden Schrägstrich, klein geschrieben (`https://chat.example.org`).
pub(crate) fn normalize_origin(url: &str) -> String {
    let url = url.trim().to_ascii_lowercase();
    match url.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or_default()),
        None => url,
    }
}

/// Darf eine Seite mit diesem `Origin` das Gateway öffnen? Browser schicken den Header immer mit; erlaubt
/// sind nur der eigene Web-Client (`public_url`) und die konfigurierten CORS-Origins (Desktop-App/Tauri,
/// Dev-Server). Ohne `Origin` (Nicht-Browser-Clients, Bots, Tests) gibt es nichts zu prüfen – fremde
/// Webseiten können so keine Verbindung mit dem Session-Cookie des Nutzers aufbauen (Cross-Site WebSocket
/// Hijacking).
pub fn origin_allowed(state: &AppState, origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let origin = normalize_origin(origin);
    normalize_origin(&state.config.public_url) == origin
        || state.config.cors_origins.iter().any(|o| normalize_origin(o) == origin)
}

pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<GatewayQuery>,
    headers: HeaderMap,
) -> Response {
    // Ein nicht lesbarer Origin zählt als fremd (leerer Text passt auf nichts).
    let origin = headers.get(header::ORIGIN).map(|v| v.to_str().unwrap_or_default());
    if !origin_allowed(&state, origin) {
        tracing::debug!(origin = ?origin, "Gateway: fremder Origin abgelehnt");
        return ApiError::Forbidden("origin_not_allowed").into_response();
    }
    // Session-Token (Cookie/Bearer) oder Bot-Token (`Authorization: Bot …`); sonst kommt es per `identify`.
    let credential = auth::credential_from_headers(&headers);
    let initial = query
        .status
        .and_then(|s| serde_json::from_value(Value::String(s)).ok())
        .unwrap_or(ChosenStatus::Online);
    ws.max_message_size(64 * 1024).on_upgrade(move |socket| async move {
        if let Err(e) = run(socket, state, credential, initial).await {
            tracing::debug!(error = %e, "Gateway-Verbindung beendet");
        }
    })
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientOp {
    /// Anmelden mit Session-Token oder Bot-Token (`Bot lykb_…` bzw. `lykb_…`).
    Identify {
        token: String,
    },
    Ping,
    /// Eigenen Status setzen (`invisible` = für andere offline).
    Presence {
        status: ChosenStatus,
    },
    /// Nutzer tippt in einem Kanal.
    Typing {
        channel_id: Uuid,
    },
}

async fn run(
    mut socket: WebSocket,
    state: AppState,
    credential: Option<auth::Credential>,
    initial: ChosenStatus,
) -> anyhow::Result<()> {
    let credential = match credential {
        Some(c) => Some(c),
        None => wait_for_identify(&mut socket).await.map(|t| auth::Credential::from_identify(&t)),
    };
    let resolved = match credential {
        Some(c) => auth::resolve_credential(&state, &c).await.map_err(api_err)?,
        None => None,
    };
    let Some(auth::AuthUser { user_id, session_id, .. }) = resolved else {
        close(&mut socket, CLOSE_UNAUTHORIZED, "unauthorized").await;
        return Ok(());
    };

    // Erst abonnieren, dann Zustand laden – so geht zwischen READY und Live-Events nichts verloren.
    let mut rx = state.events.subscribe();
    let mut conn = Connection::load(&state, user_id, session_id).await?;

    // Präsenz anmelden; ab hier muss die Verbindung in jedem Fall wieder abgemeldet werden.
    let (presence_id, change) = state.presence.connect(user_id, initial);
    if let Some(status) = change {
        conn.publish_own_presence(&state, status);
    }
    let result = serve(&mut socket, &state, &mut rx, &mut conn, presence_id).await;
    if let Some(status) = state.presence.disconnect(user_id, presence_id) {
        conn.publish_own_presence(&state, status);
    }
    result
}

/// Hauptschleife einer angemeldeten Verbindung: READY senden, dann Client-Ops und Bus-Events verarbeiten.
async fn serve(
    socket: &mut WebSocket,
    state: &AppState,
    rx: &mut Receiver<Arc<BusEvent>>,
    conn: &mut Connection,
    presence_id: u64,
) -> anyhow::Result<()> {
    let (user_id, session_id) = (conn.user_id, conn.session_id);
    let user = models::user_by_id(&state.db, user_id).await.map_err(api_err)?;
    let presences = conn.load_presences(state).await?;
    let dm_calls = calls::for_user(&state.db, user_id).await.map_err(api_err)?;
    send(
        socket,
        "READY",
        json!({
            "user": user,
            "community_ids": conn.communities,
            "dm_channel_ids": conn.dms,
            "presences": presences,
            "dm_calls": dm_calls,
        }),
    )
    .await?;

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientOp>(&text) {
                    Ok(ClientOp::Ping) => send(socket, "PONG", json!({})).await?,
                    Ok(ClientOp::Presence { status }) => {
                        if let Some(visible) = state.presence.set(user_id, presence_id, status) {
                            conn.publish_own_presence(state, visible);
                        }
                    }
                    Ok(ClientOp::Typing { channel_id }) => conn.typing(state, channel_id).await?,
                    Ok(ClientOp::Identify { .. }) | Err(_) => {}
                },
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
            },
            event = rx.recv() => match event {
                Ok(event) => match conn.handle(state, &event).await? {
                    Action::Skip => {}
                    Action::Forward => socket.send(Message::Text(event.text())).await?,
                    Action::Joined(community_id) => conn.joined(state, socket, community_id).await?,
                    Action::Close => {
                        close(socket, CLOSE_UNAUTHORIZED, "session_revoked").await;
                        break;
                    }
                },
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(%user_id, skipped = n, "Gateway hinkt hinterher – RESYNC");
                    *conn = Connection::load(state, user_id, session_id).await?;
                    // Präsenz gibt es nur über das Gateway – den aktuellen Stand gleich mitschicken.
                    let presences = conn.load_presences(state).await?;
                    let dm_calls = calls::for_user(&state.db, user_id).await.map_err(api_err)?;
                    send(socket, "RESYNC", json!({ "presences": presences, "dm_calls": dm_calls })).await?;
                }
                Err(RecvError::Closed) => break,
            },
            _ = heartbeat.tick() => {
                socket.send(Message::Ping(Default::default())).await?;
            }
        }
    }
    Ok(())
}

async fn wait_for_identify(socket: &mut WebSocket) -> Option<String> {
    let first = tokio::time::timeout(IDENTIFY_TIMEOUT, socket.recv()).await.ok()??.ok()?;
    match first {
        Message::Text(text) => match serde_json::from_str(&text).ok()? {
            ClientOp::Identify { token } => Some(token),
            _ => None,
        },
        _ => None,
    }
}

enum Action {
    Skip,
    Forward,
    /// Eigener Beitritt zu einer Community: Präsenz austauschen.
    Joined(Uuid),
    Close,
}

/// `PRESENCE_UPDATE` an alle Communities des Nutzers. Empfänger in mehreren gemeinsamen
/// Communities filtern Duplikate (siehe `Connection::last_presence`).
fn publish_presence(
    state: &AppState,
    communities: impl Iterator<Item = Uuid>,
    user_id: Uuid,
    status: Status,
) {
    for community_id in communities {
        state.events.publish(
            events::community(community_id),
            "PRESENCE_UPDATE",
            json!({ "user_id": user_id, "status": status }),
        );
    }
}

/// `PRESENCE_UPDATE` an jeden Freund einzeln (auch ohne gemeinsame Community). Doppelte Meldungen
/// filtert der Empfänger wie bei Communities.
fn publish_presence_to_friends(
    state: &AppState,
    friends: impl Iterator<Item = Uuid>,
    user_id: Uuid,
    status: Status,
) {
    for friend_id in friends {
        state.events.publish(
            Scope::User(friend_id),
            "PRESENCE_UPDATE",
            json!({ "user_id": user_id, "status": status }),
        );
    }
}

/// Zustand einer Verbindung: abonnierte Communities und gecachte Kanal-Sichtbarkeit.
struct Connection {
    user_id: Uuid,
    session_id: Uuid,
    communities: HashSet<Uuid>,
    /// DM-Kanäle, an denen der Nutzer teilnimmt.
    dms: HashSet<Uuid>,
    /// Freunde – bekommen die eigene Präsenz auch ohne gemeinsame Community.
    friends: HashSet<Uuid>,
    /// community_id → (channel_id → darf sehen). Wird bei rechte-relevanten Events verworfen.
    visible: HashMap<Uuid, HashMap<Uuid, bool>>,
    /// Zuletzt an diesen Client gemeldeter Status je Nutzer – unterdrückt doppelte `PRESENCE_UPDATE`s.
    last_presence: HashMap<Uuid, Status>,
}

impl Connection {
    async fn load(state: &AppState, user_id: Uuid, session_id: Uuid) -> anyhow::Result<Self> {
        let communities: Vec<Uuid> =
            sqlx::query_scalar("SELECT community_id FROM members WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(&state.db)
                .await?;
        let dms: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM dm_recipients WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
        let friends = relationships::friend_ids(&state.db, user_id).await.map_err(api_err)?;
        Ok(Self {
            user_id,
            session_id,
            communities: communities.into_iter().collect(),
            dms: dms.into_iter().collect(),
            friends: friends.into_iter().collect(),
            visible: HashMap::new(),
            last_presence: HashMap::new(),
        })
    }

    /// Eigene Präsenz an alle Communities und Freunde verteilen.
    fn publish_own_presence(&self, state: &AppState, status: Status) {
        publish_presence(state, self.communities.iter().copied(), self.user_id, status);
        publish_presence_to_friends(state, self.friends.iter().copied(), self.user_id, status);
    }

    /// Status aller nicht-offline Mitglieder der eigenen Communities und Freunde (für READY/RESYNC).
    /// Wer fehlt, ist offline – so bleibt READY bei vielen Mitgliedern klein.
    async fn load_presences(&mut self, state: &AppState) -> anyhow::Result<Vec<Value>> {
        let ids: Vec<Uuid> = self.communities.iter().copied().collect();
        let mut users: Vec<Uuid> =
            sqlx::query_scalar("SELECT DISTINCT user_id FROM members WHERE community_id = ANY($1)")
                .bind(&ids)
                .fetch_all(&state.db)
                .await?;
        let known: HashSet<Uuid> = users.iter().copied().collect();
        users.extend(self.friends.iter().copied().filter(|id| !known.contains(id)));
        let snapshot = state.presence.snapshot(&users);
        self.last_presence = snapshot.iter().copied().collect();
        Ok(snapshot
            .into_iter()
            .map(|(user_id, status)| json!({ "user_id": user_id, "status": status }))
            .collect())
    }

    /// Nach dem eigenen Beitritt: Status der (nicht-offline) Mitglieder als ein `PRESENCE_BULK` an
    /// diesen Client, eigenen Status an die Community.
    async fn joined(
        &mut self,
        state: &AppState,
        socket: &mut WebSocket,
        community_id: Uuid,
    ) -> anyhow::Result<()> {
        let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM members WHERE community_id = $1")
            .bind(community_id)
            .fetch_all(&state.db)
            .await?;
        let mut changed = Vec::new();
        for (user_id, status) in state.presence.snapshot(&users) {
            if user_id == self.user_id || self.last_presence.insert(user_id, status) == Some(status) {
                continue;
            }
            changed.push(json!({ "user_id": user_id, "status": status }));
        }
        if !changed.is_empty() {
            send(socket, "PRESENCE_BULK", json!({ "presences": changed })).await?;
        }
        let own = state.presence.visible(self.user_id);
        if own != Status::Offline {
            publish_presence(state, std::iter::once(community_id), self.user_id, own);
        }
        Ok(())
    }

    /// `typing`-Op: gedrosselt, nur in sichtbaren Textkanälen (inkl. DMs) mit `SEND_MESSAGES`.
    async fn typing(&self, state: &AppState, channel_id: Uuid) -> anyhow::Result<()> {
        if !state.presence.typing_allowed(self.user_id, channel_id) {
            return Ok(());
        }
        let (ctx, perms) = match permissions::channel_permissions(&state.db, self.user_id, channel_id).await {
            Ok(r) => r,
            Err(ApiError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(api_err(e)),
        };
        if !perms.contains(Permissions::SEND_MESSAGES) {
            return Ok(());
        }
        let is_text = models::channel_by_id(&state.db, channel_id)
            .await
            .map_err(api_err)?
            .is_some_and(|c| models::is_text_like(&c.kind));
        if is_text {
            state.events.publish(
                events::channel(ctx.community_id, channel_id),
                "TYPING_START",
                json!({ "channel_id": channel_id, "community_id": ctx.community_id, "user_id": self.user_id }),
            );
        }
        Ok(())
    }

    async fn handle(&mut self, state: &AppState, event: &BusEvent) -> anyhow::Result<Action> {
        if let Some(control) = event.control {
            return Ok(self.handle_control(event.scope, control));
        }
        if !event.also.is_empty() {
            // `publish_any`: höchstens einmal, wenn irgendein Scope passt.
            let any = std::iter::once(&event.scope).chain(&event.also).any(|scope| self.subscribed(*scope));
            return Ok(if any { Action::Forward } else { Action::Skip });
        }

        match event.scope {
            Scope::Instance => Ok(Action::Forward),
            Scope::Session(id) => Ok(if id == self.session_id { Action::Forward } else { Action::Skip }),
            Scope::Dm(channel_id) => {
                Ok(if self.dms.contains(&channel_id) { Action::Forward } else { Action::Skip })
            }
            Scope::User(id) => {
                if id != self.user_id {
                    return Ok(Action::Skip);
                }
                if event.event.kind == "PERMISSIONS_CHANGED" {
                    self.visible.clear();
                }
                // Präsenz von Freunden: wie bei Communities nur echte Änderungen weitergeben.
                if event.event.kind == "PRESENCE_UPDATE" && !self.presence_is_new(&event.event.data) {
                    return Ok(Action::Skip);
                }
                Ok(Action::Forward)
            }
            Scope::Community { community_id, channel_id } => {
                if !self.communities.contains(&community_id) {
                    return Ok(Action::Skip);
                }
                if event.affects_permissions {
                    self.visible.remove(&community_id);
                }
                if let Some(channel_id) = channel_id
                    && !self.can_view(state, community_id, channel_id).await?
                {
                    return Ok(Action::Skip);
                }
                if event.event.kind == "COMMUNITY_DELETE" {
                    self.communities.remove(&community_id);
                    self.visible.remove(&community_id);
                }
                if event.event.kind == "PRESENCE_UPDATE" && !self.presence_is_new(&event.event.data) {
                    return Ok(Action::Skip);
                }
                Ok(Action::Forward)
            }
        }
    }

    /// Passt der Scope auf diese Verbindung? Nur Abo-Zugehörigkeit, ohne Kanal-Sichtbarkeit
    /// (`publish_any` erlaubt keine kanalgebundenen Scopes).
    fn subscribed(&self, scope: Scope) -> bool {
        match scope {
            Scope::Instance => true,
            Scope::Session(id) => id == self.session_id,
            Scope::Dm(channel_id) => self.dms.contains(&channel_id),
            Scope::User(id) => id == self.user_id,
            Scope::Community { community_id, channel_id } => {
                channel_id.is_none() && self.communities.contains(&community_id)
            }
        }
    }

    /// Eigene Präsenz geht nicht an die eigenen Verbindungen (der Client kennt seinen Status selbst);
    /// fremde nur, wenn sie sich gegenüber dem zuletzt gemeldeten Stand geändert hat.
    fn presence_is_new(&mut self, data: &Value) -> bool {
        let user_id = data["user_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
        let status = serde_json::from_value::<Status>(data["status"].clone()).ok();
        match (user_id, status) {
            // Unbekannt = offline (READY/RESYNC schicken Offline-Nutzer nicht mit).
            (Some(user_id), Some(status)) if user_id != self.user_id => {
                self.last_presence.insert(user_id, status).unwrap_or(Status::Offline) != status
            }
            _ => false,
        }
    }

    fn handle_control(&mut self, scope: Scope, control: Control) -> Action {
        match (scope, control) {
            (Scope::Session(id), Control::SessionRevoked) if id == self.session_id => Action::Close,
            (_, Control::Joined { user_id, community_id }) if user_id == self.user_id => {
                self.communities.insert(community_id);
                self.visible.remove(&community_id);
                Action::Joined(community_id)
            }
            (_, Control::Left { user_id, community_id }) if user_id == self.user_id => {
                self.communities.remove(&community_id);
                self.visible.remove(&community_id);
                Action::Skip
            }
            (_, Control::DmJoined { user_id, channel_id }) if user_id == self.user_id => {
                self.dms.insert(channel_id);
                Action::Skip
            }
            (_, Control::DmLeft { user_id, channel_id }) if user_id == self.user_id => {
                self.dms.remove(&channel_id);
                Action::Skip
            }
            (_, Control::FriendAdded { user_id, friend_id }) if user_id == self.user_id => {
                self.friends.insert(friend_id);
                Action::Skip
            }
            (_, Control::FriendRemoved { user_id, friend_id }) if user_id == self.user_id => {
                self.friends.remove(&friend_id);
                Action::Skip
            }
            _ => Action::Skip,
        }
    }

    async fn can_view(
        &mut self,
        state: &AppState,
        community_id: Uuid,
        channel_id: Uuid,
    ) -> anyhow::Result<bool> {
        if !self.visible.contains_key(&community_id) {
            let map = match self.compute_visibility(state, community_id).await {
                Ok(map) => map,
                Err(ApiError::NotFound(_)) => {
                    // Nicht mehr Mitglied.
                    self.communities.remove(&community_id);
                    return Ok(false);
                }
                Err(e) => return Err(api_err(e)),
            };
            self.visible.insert(community_id, map);
        }
        Ok(self.visible.get(&community_id).and_then(|m| m.get(&channel_id)).copied().unwrap_or(false))
    }

    async fn compute_visibility(
        &self,
        state: &AppState,
        community_id: Uuid,
    ) -> Result<HashMap<Uuid, bool>, ApiError> {
        let ctx = permissions::member_context(&state.db, self.user_id, community_id).await?;
        let overrides = permissions::community_overrides(&state.db, community_id).await?;
        let channels: Vec<(Uuid, Option<Uuid>)> =
            sqlx::query_as("SELECT id, category_id FROM channels WHERE community_id = $1")
                .bind(community_id)
                .fetch_all(&state.db)
                .await?;
        let mut visible: HashMap<Uuid, bool> = HashMap::new();
        let mut visible_categories = Vec::new();
        for (id, category_id) in channels {
            let perms = ctx.in_channel(overrides.get(&id).map_or(&[], Vec::as_slice));
            let can_view = perms.contains(Permissions::VIEW_CHANNEL);
            if can_view {
                visible_categories.push(category_id);
            }
            visible.insert(id, can_view);
        }
        // Kategorien laufen unter ihrer ID durch dieselbe Prüfung (Events `CATEGORY_*`).
        let categories: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM channel_categories WHERE community_id = $1")
                .bind(community_id)
                .fetch_all(&state.db)
                .await?;
        let can_manage = ctx.base().contains(Permissions::MANAGE_CHANNELS);
        for id in categories {
            let can_view = categories::is_visible(id, can_manage, visible_categories.iter().copied());
            visible.insert(id, can_view);
        }
        Ok(visible)
    }
}

async fn send(socket: &mut WebSocket, kind: &str, data: Value) -> anyhow::Result<()> {
    let text = serde_json::to_string(&json!({ "t": kind, "d": data }))?;
    socket.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ =
        socket.send(Message::Close(Some(CloseFrame { code, reason: Utf8Bytes::from_static(reason) }))).await;
}

fn api_err(e: ApiError) -> anyhow::Error {
    anyhow::anyhow!(e)
}
