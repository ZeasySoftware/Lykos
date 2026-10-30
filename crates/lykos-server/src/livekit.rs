//! LiveKit-Anbindung: Access-Tokens (JWT HS256), Prüfung signierter Webhooks und die
//! RoomService-API (Twirp über HTTP).
//!
//! Token-Format laut LiveKit-Doku: `iss` = API-Key, `sub` = Identity, `nbf`/`exp`, `name` und
//! der `video`-Grant. Fehlende `canPublish`/`canSubscribe`/`canPublishData` bedeuten bei LiveKit
//! „erlaubt“ – deshalb setzen wir sie immer explizit.

use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{config::LiveKitConfig, permissions::Permissions};

/// Gültigkeit des Join-Tokens. Danach verlängert LiveKit die Sitzung selbst (Refresh-Tokens).
pub const JOIN_TOKEN_TTL: i64 = 10 * 60;
/// Gültigkeit der Admin-Tokens für RoomService-Aufrufe.
const ADMIN_TOKEN_TTL: i64 = 60;
/// Toleranz für Uhrenabweichung bei `nbf`/`exp`.
const LEEWAY: i64 = 60;

/// Quellen, die ein Teilnehmer veröffentlichen darf (Werte wie in LiveKits `TrackSource`).
pub const SOURCE_MICROPHONE: &str = "microphone";
pub const SOURCE_CAMERA: &str = "camera";
pub const SOURCE_SCREEN_SHARE: &str = "screen_share";
pub const SOURCE_SCREEN_SHARE_AUDIO: &str = "screen_share_audio";

fn is_false(b: &bool) -> bool {
    !*b
}

/// LiveKits `video`-Grant (nur die Felder, die Lykos braucht).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoGrant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub room_join: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub room_list: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub room_create: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub room_admin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_publish: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_subscribe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_publish_data: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub can_publish_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub: String,
    pub nbf: i64,
    pub exp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoGrant>,
    /// Base64-SHA-256 des Request-Bodys (nur bei Webhooks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

// --- JWT --------------------------------------------------------------------

fn mac(secret: &str) -> Hmac<Sha256> {
    // HMAC akzeptiert Schlüssel jeder Länge.
    Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC-Schlüssel")
}

/// Signiert Claims als JWT (HS256).
pub fn sign(claims: &Claims, secret: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("Claims serialisierbar"));
    let signing_input = format!("{header}.{payload}");
    let mut mac = mac(secret);
    mac.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig}")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("Token fehlerhaft")]
    Malformed,
    #[error("Signatur ungültig")]
    BadSignature,
    #[error("Token abgelaufen oder noch nicht gültig")]
    Expired,
    #[error("falscher Aussteller")]
    WrongIssuer,
    #[error("Body-Hash stimmt nicht")]
    BodyMismatch,
}

/// Prüft ein HS256-JWT: Signatur (konstantzeitig), Algorithmus, Gültigkeit und Aussteller.
pub fn verify(token: &str, api_key: &str, secret: &str) -> Result<Claims, TokenError> {
    let (signing_input, sig) = token.rsplit_once('.').ok_or(TokenError::Malformed)?;
    let (header, payload) = signing_input.split_once('.').ok_or(TokenError::Malformed)?;
    if payload.contains('.') {
        return Err(TokenError::Malformed);
    }

    let header: Value = URL_SAFE_NO_PAD
        .decode(header)
        .ok()
        .and_then(|h| serde_json::from_slice(&h).ok())
        .ok_or(TokenError::Malformed)?;
    if header.get("alg").and_then(Value::as_str) != Some("HS256") {
        return Err(TokenError::Malformed);
    }

    let sig = URL_SAFE_NO_PAD.decode(sig).map_err(|_| TokenError::Malformed)?;
    let mut mac = mac(secret);
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&sig).map_err(|_| TokenError::BadSignature)?;

    let claims: Claims = URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|p| serde_json::from_slice(&p).ok())
        .ok_or(TokenError::Malformed)?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    if claims.exp + LEEWAY < now || claims.nbf - LEEWAY > now {
        return Err(TokenError::Expired);
    }
    if claims.iss != api_key {
        return Err(TokenError::WrongIssuer);
    }
    Ok(claims)
}

/// Prüft den `Authorization`-Header eines LiveKit-Webhooks: signiertes JWT, dessen
/// `sha256`-Claim der Base64-SHA-256 des Bodys ist.
pub fn verify_webhook(cfg: &LiveKitConfig, authorization: &str, body: &[u8]) -> Result<(), TokenError> {
    let token = authorization.trim();
    let token = token.strip_prefix("Bearer ").unwrap_or(token).trim();
    let claims = verify(token, &cfg.api_key, &cfg.api_secret)?;
    let expected = STANDARD.encode(Sha256::digest(body));
    let got = claims.sha256.ok_or(TokenError::BodyMismatch)?;
    // Manche SDKs lassen das Padding weg.
    if got.trim_end_matches('=') != expected.trim_end_matches('=') {
        return Err(TokenError::BodyMismatch);
    }
    Ok(())
}

// --- Tokens -----------------------------------------------------------------

fn claims(cfg: &LiveKitConfig, sub: String, ttl: i64, video: VideoGrant) -> Claims {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    Claims {
        iss: cfg.api_key.clone(),
        sub,
        nbf: now,
        exp: now + ttl,
        name: None,
        video: Some(video),
        sha256: None,
    }
}

/// Quellen, die ein Teilnehmer mit diesen Kanalrechten veröffentlichen darf: Mikrofon mit `SPEAK`,
/// Bildschirm (+Ton) mit `STREAM`, Kamera mit `VIEW_CHANNEL` + `CONNECT` + `VIDEO`.
pub fn allowed_sources(perms: Permissions) -> Vec<&'static str> {
    let mut sources = Vec::new();
    if perms.contains(Permissions::SPEAK) {
        sources.push(SOURCE_MICROPHONE);
    }
    if perms.contains(Permissions::STREAM) {
        sources.push(SOURCE_SCREEN_SHARE);
        sources.push(SOURCE_SCREEN_SHARE_AUDIO);
    }
    if perms.contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::VIDEO) {
        sources.push(SOURCE_CAMERA);
    }
    sources
}

/// Grant für einen Teilnehmer: hören immer, veröffentlichen laut `allowed_sources`.
/// Der Datenkanal bleibt vorerst gesperrt.
pub fn participant_grant(room: Uuid, perms: Permissions) -> VideoGrant {
    let sources: Vec<String> = allowed_sources(perms).into_iter().map(str::to_owned).collect();
    VideoGrant {
        room: Some(room.to_string()),
        room_join: true,
        can_publish: Some(!sources.is_empty()),
        can_subscribe: Some(true),
        can_publish_data: Some(false),
        can_publish_sources: sources,
        ..Default::default()
    }
}

/// Access-Token zum Betreten eines Voice-Kanals (Raum = Kanal-ID, Identity = Nutzer-ID).
pub fn join_token(cfg: &LiveKitConfig, user_id: Uuid, name: &str, room: Uuid, perms: Permissions) -> String {
    let mut c = claims(cfg, user_id.to_string(), JOIN_TOKEN_TTL, participant_grant(room, perms));
    c.name = Some(name.to_owned());
    sign(&c, &cfg.api_secret)
}

fn admin_token(cfg: &LiveKitConfig, room: Option<&str>) -> String {
    let video = VideoGrant {
        room: room.map(str::to_owned),
        room_admin: true,
        room_list: true,
        room_create: true,
        ..Default::default()
    };
    sign(&claims(cfg, String::new(), ADMIN_TOKEN_TTL, video), &cfg.api_secret)
}

// --- Webhook-Payload --------------------------------------------------------

/// Webhook-Event (protojson, camelCase). Nur die Felder, die Lykos auswertet.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookEvent {
    pub event: String,
    pub room: Option<RoomInfo>,
    pub participant: Option<ParticipantInfo>,
    pub track: Option<TrackInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RoomInfo {
    pub name: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct ParticipantInfo {
    pub sid: String,
    pub identity: String,
    pub tracks: Vec<TrackInfo>,
    /// Aktuelle Rechte des Teilnehmers in LiveKit (fehlt in älteren/handgebauten Payloads).
    pub permission: Option<ParticipantPermission>,
}

/// LiveKits `ParticipantPermission` (protojson lässt `false`/leere Felder weg).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParticipantPermission {
    pub can_publish: bool,
    /// `TrackSource`-Werte als Name oder Zahl.
    pub can_publish_sources: Vec<Value>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct TrackInfo {
    pub sid: String,
    /// `TrackSource` als Name (`"SCREEN_SHARE"`) oder Zahl (3), je nach Serialisierung.
    pub source: Value,
    pub muted: bool,
}

/// `TrackSource` (Name oder Zahl) → Quellenname wie im Token (`"screen_share"`).
fn source_key(source: &Value) -> Option<&'static str> {
    let n = match source {
        Value::String(s) => match s.to_ascii_uppercase().as_str() {
            "CAMERA" => 1,
            "MICROPHONE" => 2,
            "SCREEN_SHARE" => 3,
            "SCREEN_SHARE_AUDIO" => 4,
            _ => return None,
        },
        Value::Number(n) => n.as_u64()?,
        _ => return None,
    };
    match n {
        1 => Some(SOURCE_CAMERA),
        2 => Some(SOURCE_MICROPHONE),
        3 => Some(SOURCE_SCREEN_SHARE),
        4 => Some(SOURCE_SCREEN_SHARE_AUDIO),
        _ => None,
    }
}

/// Quellenname → `TrackSource`-Enum-Name für die RoomService-API (protojson).
fn source_enum(key: &str) -> &'static str {
    match key {
        SOURCE_CAMERA => "CAMERA",
        SOURCE_MICROPHONE => "MICROPHONE",
        SOURCE_SCREEN_SHARE => "SCREEN_SHARE",
        SOURCE_SCREEN_SHARE_AUDIO => "SCREEN_SHARE_AUDIO",
        _ => "UNKNOWN",
    }
}

impl TrackInfo {
    pub fn source_key(&self) -> Option<&'static str> {
        source_key(&self.source)
    }

    pub fn is_screen_share(&self) -> bool {
        self.source_key() == Some(SOURCE_SCREEN_SHARE)
    }

    pub fn is_camera(&self) -> bool {
        self.source_key() == Some(SOURCE_CAMERA)
    }
}

impl ParticipantPermission {
    /// Entsprechen die Veröffentlichungsrechte genau `wanted`?
    fn publishes_exactly(&self, wanted: &[&str]) -> bool {
        if wanted.is_empty() {
            return !self.can_publish;
        }
        let mut have: Vec<&str> = self.can_publish_sources.iter().filter_map(source_key).collect();
        have.sort_unstable();
        have.dedup();
        let mut want = wanted.to_vec();
        want.sort_unstable();
        // Leere Liste bei `canPublish` heißt in LiveKit „alle Quellen“ – also nie gleich.
        self.can_publish && have == want && self.can_publish_sources.len() == have.len()
    }
}

impl ParticipantInfo {
    /// Veröffentlichte, nicht stummgeschaltete Spuren, deren Quelle nicht (mehr) erlaubt ist.
    fn forbidden_tracks<'a>(&'a self, wanted: &'a [&str]) -> impl Iterator<Item = &'a TrackInfo> + 'a {
        self.tracks.iter().filter(move |t| !t.muted && t.source_key().is_some_and(|k| !wanted.contains(&k)))
    }

    /// Weichen die LiveKit-Rechte oder veröffentlichten Spuren von den Kanalrechten ab?
    /// Ohne bekannte Rechte (`permission` fehlt) wird nur auf verbotene Spuren geprüft.
    pub fn needs_permission_sync(&self, perms: Permissions) -> bool {
        let wanted = allowed_sources(perms);
        let permission_differs = self.permission.as_ref().is_some_and(|p| !p.publishes_exactly(&wanted));
        permission_differs || self.forbidden_tracks(&wanted).next().is_some()
    }
}

// --- RoomService (Twirp) ----------------------------------------------------

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        crate::install_crypto_provider();
        reqwest::Client::builder().timeout(Duration::from_secs(5)).build().expect("HTTP-Client")
    })
}

/// Ruft eine RoomService-Methode auf. `Ok(None)` bei Twirp-`not_found`.
async fn twirp(cfg: &LiveKitConfig, method: &str, room: Option<&str>, body: Value) -> Result<Option<Value>> {
    let url = format!("{}/twirp/livekit.RoomService/{method}", cfg.api_url);
    let res = http()
        .post(&url)
        .bearer_auth(admin_token(cfg, room))
        .json(&body)
        .send()
        .await
        .with_context(|| format!("LiveKit {method} nicht erreichbar"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        anyhow::bail!("LiveKit {method}: {status} {text}");
    }
    Ok(Some(res.json().await.with_context(|| format!("LiveKit {method}: ungültige Antwort"))?))
}

/// Wirft einen Teilnehmer aus dem Raum. Nicht (mehr) vorhanden ist kein Fehler.
pub async fn remove_participant(cfg: &LiveKitConfig, room: Uuid, identity: Uuid) -> Result<()> {
    let room = room.to_string();
    twirp(cfg, "RemoveParticipant", Some(&room), json!({ "room": room, "identity": identity.to_string() }))
        .await?;
    Ok(())
}

/// Schließt einen Raum und trennt alle Teilnehmer.
pub async fn delete_room(cfg: &LiveKitConfig, room: Uuid) -> Result<()> {
    let room = room.to_string();
    twirp(cfg, "DeleteRoom", Some(&room), json!({ "room": room })).await?;
    Ok(())
}

/// Namen aller aktiven Räume.
pub async fn list_rooms(cfg: &LiveKitConfig) -> Result<Vec<String>> {
    let res = twirp(cfg, "ListRooms", None, json!({})).await?.unwrap_or_default();
    Ok(res["rooms"]
        .as_array()
        .map(|rooms| rooms.iter().filter_map(|r| r["name"].as_str().map(str::to_owned)).collect())
        .unwrap_or_default())
}

/// Ein Teilnehmer; `None`, wenn er (nicht mehr) im Raum ist.
pub async fn get_participant(
    cfg: &LiveKitConfig,
    room: Uuid,
    identity: Uuid,
) -> Result<Option<ParticipantInfo>> {
    let room = room.to_string();
    let identity = identity.to_string();
    let res =
        twirp(cfg, "GetParticipant", Some(&room), json!({ "room": room, "identity": identity })).await?;
    Ok(res.and_then(|v| serde_json::from_value::<ParticipantInfo>(v).ok()).filter(|p| p.identity == identity))
}

/// Veröffentlichungsrechte eines verbundenen Teilnehmers an die Kanalrechte angleichen
/// (`UpdateParticipant` ersetzt die komplette `permission`, daher alle Felder explizit).
pub async fn update_permission(
    cfg: &LiveKitConfig,
    room: Uuid,
    identity: Uuid,
    perms: Permissions,
) -> Result<()> {
    let room = room.to_string();
    let sources: Vec<&str> = allowed_sources(perms).into_iter().map(source_enum).collect();
    let body = json!({
        "room": room,
        "identity": identity.to_string(),
        "permission": {
            "canSubscribe": true,
            "canPublish": !sources.is_empty(),
            "canPublishData": false,
            "canPublishSources": sources,
        },
    });
    twirp(cfg, "UpdateParticipant", Some(&room), body).await?;
    Ok(())
}

/// Veröffentlichte Spur serverseitig stummschalten.
pub async fn mute_track(cfg: &LiveKitConfig, room: Uuid, identity: Uuid, track_sid: &str) -> Result<()> {
    let room = room.to_string();
    let body =
        json!({ "room": room, "identity": identity.to_string(), "trackSid": track_sid, "muted": true });
    twirp(cfg, "MutePublishedTrack", Some(&room), body).await?;
    Ok(())
}

/// Rechte eines verbundenen Teilnehmers im laufenden Call angleichen: `UpdateParticipant`, falls
/// die LiveKit-Rechte abweichen, danach Spuren entzogener Quellen per `MutePublishedTrack`
/// stummschalten (LiveKit beendet sie beim Rechteentzug je nach Version selbst; doppelt hält besser).
/// `known` = bereits bekannter Teilnehmerzustand (Webhook), sonst wird er abgefragt.
/// `Ok(false)`, wenn der Teilnehmer nicht (mehr) im Raum ist.
pub async fn sync_permissions(
    cfg: &LiveKitConfig,
    room: Uuid,
    identity: Uuid,
    perms: Permissions,
    known: Option<ParticipantInfo>,
) -> Result<bool> {
    let info = match known {
        Some(info) => info,
        None => match get_participant(cfg, room, identity).await? {
            Some(info) => info,
            None => return Ok(false),
        },
    };
    let wanted = allowed_sources(perms);
    if info.permission.as_ref().is_none_or(|p| !p.publishes_exactly(&wanted)) {
        update_permission(cfg, room, identity, perms).await?;
    }
    for track in info.forbidden_tracks(&wanted) {
        mute_track(cfg, room, identity, &track.sid).await?;
    }
    Ok(true)
}

/// Teilnehmer eines Raums.
pub async fn list_participants(cfg: &LiveKitConfig, room: &str) -> Result<Vec<ParticipantInfo>> {
    let res = twirp(cfg, "ListParticipants", Some(room), json!({ "room": room })).await?.unwrap_or_default();
    Ok(serde_json::from_value(res["participants"].clone()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LiveKitConfig {
        LiveKitConfig::for_tests()
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let c = cfg();
        let token = join_token(&c, Uuid::now_v7(), "Alex", Uuid::now_v7(), Permissions::DEFAULT_ROLE);
        let claims = verify(&token, &c.api_key, &c.api_secret).unwrap();
        let video = claims.video.unwrap();
        assert!(video.room_join);
        assert_eq!(video.can_publish, Some(true));
        assert_eq!(video.can_publish_data, Some(false));
        assert_eq!(video.can_publish_sources, ["microphone", "screen_share", "screen_share_audio", "camera"]);
        assert_eq!(claims.name.as_deref(), Some("Alex"));

        assert_eq!(verify(&token, &c.api_key, "anderes-secret"), Err(TokenError::BadSignature));
        assert_eq!(verify(&token, "anderer-key", &c.api_secret), Err(TokenError::WrongIssuer));
        assert_eq!(verify("a.b", &c.api_key, &c.api_secret), Err(TokenError::Malformed));
    }

    #[test]
    fn listen_only_cannot_publish() {
        let g = participant_grant(Uuid::now_v7(), Permissions::CONNECT);
        assert_eq!(g.can_publish, Some(false));
        assert!(g.can_publish_sources.is_empty());
        let json = serde_json::to_value(&g).unwrap();
        assert_eq!(json["canPublish"], false);
        assert_eq!(json["canSubscribe"], true);
        assert_eq!(json["roomJoin"], true);
    }

    #[test]
    fn camera_needs_view_connect_and_video() {
        let room = Uuid::now_v7();
        let g =
            participant_grant(room, Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::VIDEO);
        assert_eq!(g.can_publish, Some(true));
        assert_eq!(g.can_publish_sources, ["camera"]);
        let g = participant_grant(room, Permissions::CONNECT | Permissions::VIDEO);
        assert_eq!(g.can_publish, Some(false));
        assert!(g.can_publish_sources.is_empty());
    }

    #[test]
    fn expired_tokens_are_rejected() {
        let c = cfg();
        let mut claims = claims(&c, "x".into(), 10, VideoGrant::default());
        claims.exp -= 3600;
        claims.nbf -= 3600;
        let token = sign(&claims, &c.api_secret);
        assert_eq!(verify(&token, &c.api_key, &c.api_secret), Err(TokenError::Expired));
    }

    #[test]
    fn webhook_body_hash() {
        let c = cfg();
        let body = br#"{"event":"room_finished"}"#;
        let mut claims = claims(&c, String::new(), 300, VideoGrant::default());
        claims.video = None;
        claims.sha256 = Some(STANDARD.encode(Sha256::digest(body)));
        let token = sign(&claims, &c.api_secret);
        assert_eq!(verify_webhook(&c, &token, body), Ok(()));
        assert_eq!(verify_webhook(&c, &format!("Bearer {token}"), body), Ok(()));
        assert_eq!(verify_webhook(&c, &token, b"{}"), Err(TokenError::BodyMismatch));
    }

    #[test]
    fn permission_sync_detection() {
        let all = Permissions::DEFAULT_ROLE;
        let info: ParticipantInfo = serde_json::from_value(json!({
            "sid": "PA_1",
            "identity": "x",
            "permission": {
                "canSubscribe": true,
                "canPublish": true,
                "canPublishSources": ["MICROPHONE", "SCREEN_SHARE", "SCREEN_SHARE_AUDIO", "CAMERA"],
            },
            "tracks": [
                { "sid": "TR_MIC", "source": "MICROPHONE" },
                { "sid": "TR_SCREEN", "source": 3 },
            ],
        }))
        .unwrap();
        assert!(!info.needs_permission_sync(all));

        // STREAM entzogen: Rechte weichen ab, Bildschirmspur ist verboten.
        let no_stream = Permissions(all.0 & !Permissions::STREAM.0);
        assert!(info.needs_permission_sync(no_stream));
        let wanted = allowed_sources(no_stream);
        let forbidden: Vec<_> = info.forbidden_tracks(&wanted).map(|t| t.sid.as_str()).collect();
        assert_eq!(forbidden, ["TR_SCREEN"]);

        // Reihenfolge und Zahlen-Enums spielen keine Rolle.
        let p = ParticipantPermission {
            can_publish: true,
            can_publish_sources: vec![json!(2), json!("CAMERA"), json!(4), json!("SCREEN_SHARE")],
        };
        assert!(p.publishes_exactly(&allowed_sources(all)));
        // `canPublish` ohne Quellenliste heißt „alles“ – nie gleich einer konkreten Liste.
        let p = ParticipantPermission { can_publish: true, can_publish_sources: vec![] };
        assert!(!p.publishes_exactly(&allowed_sources(all)));
        // Nur zuhören: canPublish = false genügt.
        let p = ParticipantPermission::default();
        assert!(p.publishes_exactly(&allowed_sources(Permissions::CONNECT)));

        // Stummgeschaltete Spuren sind kein Grund zum Eingreifen; ohne `permission` zählen nur Spuren.
        let info: ParticipantInfo = serde_json::from_value(json!({
            "identity": "x",
            "tracks": [{ "sid": "TR_MIC", "source": "MICROPHONE", "muted": true }],
        }))
        .unwrap();
        assert!(!info.needs_permission_sync(Permissions::CONNECT));
    }

    #[test]
    fn track_source_parsing() {
        let t: TrackInfo =
            serde_json::from_value(json!({ "sid": "TR_1", "source": "SCREEN_SHARE" })).unwrap();
        assert!(t.is_screen_share());
        let t: TrackInfo = serde_json::from_value(json!({ "sid": "TR_1", "source": 3 })).unwrap();
        assert!(t.is_screen_share());
        let t: TrackInfo = serde_json::from_value(json!({ "sid": "TR_1", "source": "MICROPHONE" })).unwrap();
        assert!(!t.is_screen_share());
        assert!(!t.is_camera());
        let t: TrackInfo = serde_json::from_value(json!({ "sid": "TR_1", "source": "CAMERA" })).unwrap();
        assert!(t.is_camera());
        let t: TrackInfo = serde_json::from_value(json!({ "sid": "TR_1", "source": 1 })).unwrap();
        assert!(t.is_camera());
        assert!(!t.is_screen_share());
    }
}
