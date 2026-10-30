//! Rechte als Bitflags und deren Auflösung (Rollen + Kanal-Overrides).
//! Der Algorithmus ist in `docs/api.md` beschrieben.

use std::ops::{BitAnd, BitOr, Not};

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permissions(pub i64);

impl Permissions {
    pub const NONE: Self = Self(0);
    pub const VIEW_CHANNEL: Self = Self(1 << 0);
    pub const SEND_MESSAGES: Self = Self(1 << 1);
    pub const MANAGE_MESSAGES: Self = Self(1 << 2);
    pub const ATTACH_FILES: Self = Self(1 << 3);
    pub const CONNECT: Self = Self(1 << 4);
    pub const SPEAK: Self = Self(1 << 5);
    pub const STREAM: Self = Self(1 << 6);
    pub const CREATE_INVITES: Self = Self(1 << 7);
    pub const MANAGE_CHANNELS: Self = Self(1 << 8);
    pub const MANAGE_ROLES: Self = Self(1 << 9);
    pub const KICK_MEMBERS: Self = Self(1 << 10);
    pub const MANAGE_COMMUNITY: Self = Self(1 << 11);
    pub const VIEW_AUDIT_LOG: Self = Self(1 << 12);
    pub const VIDEO: Self = Self(1 << 13);
    pub const ADD_REACTIONS: Self = Self(1 << 14);
    pub const MENTION_EVERYONE: Self = Self(1 << 15);
    /// Custom-Emotes der Community hochladen, umbenennen und löschen.
    pub const MANAGE_EMOJIS: Self = Self(1 << 16);
    /// Mitglieder bannen und Banns aufheben (Bannliste einsehen).
    pub const BAN_MEMBERS: Self = Self(1 << 17);
    /// Webhooks der Textkanäle anlegen, ändern, Token neu erzeugen und löschen.
    pub const MANAGE_WEBHOOKS: Self = Self(1 << 18);
    pub const ADMINISTRATOR: Self = Self(1 << 20);
    /// Eigenen Nickname in der Community ändern.
    pub const CHANGE_NICKNAME: Self = Self(1 << 21);
    /// Nicknames anderer Mitglieder ändern.
    pub const MANAGE_NICKNAMES: Self = Self(1 << 22);

    /// Alle definierten Rechte.
    pub const ALL: Self = Self(
        ((1 << 17) - 1)
            | Self::BAN_MEMBERS.0
            | Self::MANAGE_WEBHOOKS.0
            | (1 << 20)
            | Self::CHANGE_NICKNAME.0
            | Self::MANAGE_NICKNAMES.0,
    );

    /// Rechte der Standardrolle neuer Communities.
    pub const DEFAULT_ROLE: Self = Self(
        Self::VIEW_CHANNEL.0
            | Self::SEND_MESSAGES.0
            | Self::ATTACH_FILES.0
            | Self::CONNECT.0
            | Self::SPEAK.0
            | Self::STREAM.0
            | Self::VIDEO.0
            | Self::ADD_REACTIONS.0
            | Self::CREATE_INVITES.0
            | Self::CHANGE_NICKNAME.0,
    );

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Nur bekannte Bits behalten (schützt vor Müll aus Clients).
    pub fn sanitized(self) -> Self {
        self & Self::ALL
    }
}

impl BitOr for Permissions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for Permissions {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl Not for Permissions {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// Alles, was zur Rechte-Berechnung eines Mitglieds in einer Community nötig ist.
#[derive(Debug, Clone)]
pub struct MemberContext {
    pub community_id: Uuid,
    pub is_owner: bool,
    pub default_role_id: Uuid,
    /// Alle Rollen des Mitglieds inkl. Standardrolle, mit ihren Rechten.
    pub roles: Vec<(Uuid, Permissions)>,
}

#[derive(Debug, Clone, Copy)]
pub struct Override {
    pub role_id: Uuid,
    pub allow: Permissions,
    pub deny: Permissions,
}

impl MemberContext {
    /// Rechte auf Community-Ebene.
    pub fn base(&self) -> Permissions {
        if self.is_owner {
            return Permissions::ALL;
        }
        let base = self.roles.iter().fold(Permissions::NONE, |acc, (_, p)| acc | *p);
        if base.contains(Permissions::ADMINISTRATOR) { Permissions::ALL } else { base }
    }

    /// Rechte in einem Kanal mit den gegebenen Overrides.
    pub fn in_channel(&self, overrides: &[Override]) -> Permissions {
        let mut perms = self.base();
        if perms.contains(Permissions::ADMINISTRATOR) {
            return Permissions::ALL;
        }

        if let Some(o) = overrides.iter().find(|o| o.role_id == self.default_role_id) {
            perms = (perms & !o.deny) | o.allow;
        }

        let (allow, deny) = overrides
            .iter()
            .filter(|o| o.role_id != self.default_role_id)
            .filter(|o| self.roles.iter().any(|(id, _)| *id == o.role_id))
            .fold((Permissions::NONE, Permissions::NONE), |(a, d), o| (a | o.allow, d | o.deny));

        (perms & !deny) | allow
    }
}

/// Lädt den Rechte-Kontext; `NotFound`, wenn der Nutzer kein Mitglied ist
/// (bewusst nicht `Forbidden`, um die Existenz fremder Communities nicht zu verraten).
pub async fn member_context(db: &PgPool, user_id: Uuid, community_id: Uuid) -> ApiResult<MemberContext> {
    let owner: Option<(Uuid,)> = sqlx::query_as(
        "SELECT c.owner_id FROM communities c
         JOIN members m ON m.community_id = c.id AND m.user_id = $2
         WHERE c.id = $1",
    )
    .bind(community_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let Some((owner_id,)) = owner else {
        return Err(ApiError::not_found());
    };

    let rows: Vec<(Uuid, i64, bool)> = sqlx::query_as(
        "SELECT r.id, r.permissions, r.is_default FROM roles r
         WHERE r.community_id = $1
           AND (r.is_default OR r.id IN (
                SELECT role_id FROM member_roles WHERE community_id = $1 AND user_id = $2))",
    )
    .bind(community_id)
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let default_role_id = rows.iter().find(|(_, _, d)| *d).map(|(id, _, _)| *id).ok_or_else(|| {
        ApiError::Internal(anyhow::anyhow!("Community {community_id} hat keine Standardrolle"))
    })?;

    Ok(MemberContext {
        community_id,
        is_owner: owner_id == user_id,
        default_role_id,
        roles: rows.into_iter().map(|(id, p, _)| (id, Permissions(p))).collect(),
    })
}

/// Overrides aller Kanäle einer Community, gruppiert nach Kanal.
pub async fn community_overrides(
    db: &PgPool,
    community_id: Uuid,
) -> ApiResult<std::collections::HashMap<Uuid, Vec<Override>>> {
    let rows: Vec<(Uuid, Uuid, i64, i64)> = sqlx::query_as(
        "SELECT o.channel_id, o.role_id, o.allow, o.deny FROM channel_overrides o
         JOIN channels c ON c.id = o.channel_id WHERE c.community_id = $1",
    )
    .bind(community_id)
    .fetch_all(db)
    .await?;

    let mut map: std::collections::HashMap<Uuid, Vec<Override>> = Default::default();
    for (channel_id, role_id, allow, deny) in rows {
        map.entry(channel_id).or_default().push(Override {
            role_id,
            allow: Permissions(allow),
            deny: Permissions(deny),
        });
    }
    Ok(map)
}

/// Rechte in DM-Kanälen: Teilnehmer dürfen lesen, schreiben, Dateien anhängen und reagieren – mehr nicht
/// (keine Moderation fremder Nachrichten, kein `@everyone`, keine Kanal-/Rollenverwaltung).
/// Anpinnen dürfen Teilnehmer ebenfalls (wie Schreiben, also nicht in blockierten DMs), das prüft
/// `routes::pins` gesondert.
pub const DM_PERMISSIONS: Permissions = Permissions(
    Permissions::VIEW_CHANNEL.0
        | Permissions::SEND_MESSAGES.0
        | Permissions::ATTACH_FILES.0
        | Permissions::ADD_REACTIONS.0,
);

/// Rechte in einer 1:1-DM, in der einer den anderen blockiert hat: Verlauf lesen, sonst nichts.
pub const BLOCKED_DM_PERMISSIONS: Permissions = Permissions::VIEW_CHANNEL;

/// Rechte in einem DM-Anruf (LiveKit-Raum des DM-Kanals): beitreten, sprechen, Bildschirm und Kamera teilen.
/// Gelten für alle Teilnehmer gleich; Nutzerlimit und Ton-Einstellungen von Sprachkanälen gibt es hier nicht.
pub const DM_CALL_PERMISSIONS: Permissions = Permissions(
    Permissions::VIEW_CHANNEL.0
        | Permissions::CONNECT.0
        | Permissions::SPEAK.0
        | Permissions::STREAM.0
        | Permissions::VIDEO.0,
);

/// Anrufrechte im DM-Kanal `channel_id` (Art `kind`): `DM_CALL_PERMISSIONS` für Teilnehmer, in einer 1:1-DM
/// mit Blockade (egal in welche Richtung) keine. Wer nicht teilnimmt, bekommt `NotFound`.
pub async fn dm_call_permissions(
    db: &PgPool,
    user_id: Uuid,
    channel_id: Uuid,
    kind: &str,
) -> ApiResult<Permissions> {
    if !is_dm_recipient(db, user_id, channel_id).await? {
        return Err(ApiError::not_found());
    }
    Ok(if kind == "dm" && dm_blocked(db, user_id, channel_id).await? {
        Permissions::NONE
    } else {
        DM_CALL_PERMISSIONS
    })
}

/// Hat in diesem DM-Kanal der Nutzer einen anderen Teilnehmer blockiert oder umgekehrt?
async fn dm_blocked(db: &PgPool, user_id: Uuid, channel_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM dm_recipients o JOIN relationships r ON r.kind = 'blocked'
                          AND ((r.user_id = $2 AND r.target_id = o.user_id)
                            OR (r.user_id = o.user_id AND r.target_id = $2))
                        WHERE o.channel_id = $1 AND o.user_id <> $2)",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(db)
    .await?)
}

/// Kontext eines Kanalzugriffs: Community-Kanal (mit Rechte-Kontext des Mitglieds) oder DM.
#[derive(Debug, Clone)]
pub struct ChannelContext {
    /// `None` bei DM-Kanälen.
    pub community_id: Option<Uuid>,
    /// Rechte-Kontext in der Community; `None` bei DM-Kanälen.
    pub member: Option<MemberContext>,
}

impl ChannelContext {
    /// Community des Kanals; DM-Kanäle haben keine → `Forbidden` (Community-Aktionen sind dort nicht möglich).
    pub fn community(&self) -> ApiResult<Uuid> {
        self.community_id.ok_or_else(ApiError::forbidden)
    }

    pub fn is_dm(&self) -> bool {
        self.community_id.is_none()
    }
}

/// Ist der Nutzer Teilnehmer des DM-Kanals?
pub async fn is_dm_recipient(db: &PgPool, user_id: Uuid, channel_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM dm_recipients WHERE channel_id = $1 AND user_id = $2)",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(db)
    .await?)
}

/// Rechte des Nutzers in einem Kanal (Community-Kanal oder DM) samt Kontext.
/// `NotFound`, wenn der Kanal nicht existiert, der Nutzer kein Mitglied/Teilnehmer ist oder ihn
/// nicht sehen darf.
pub async fn channel_permissions(
    db: &PgPool,
    user_id: Uuid,
    channel_id: Uuid,
) -> ApiResult<(ChannelContext, Permissions)> {
    let community: Option<(Option<Uuid>, String)> =
        sqlx::query_as("SELECT community_id, kind FROM channels WHERE id = $1")
            .bind(channel_id)
            .fetch_optional(db)
            .await?;
    let Some((community_id, kind)) = community else {
        return Err(ApiError::not_found());
    };
    let Some(community_id) = community_id else {
        // DM-Kanal: nur Teilnehmer.
        if !is_dm_recipient(db, user_id, channel_id).await? {
            return Err(ApiError::not_found());
        }
        // 1:1-DM mit Blockade (egal in welche Richtung): nur noch lesen.
        let perms = if kind == "dm" && dm_blocked(db, user_id, channel_id).await? {
            BLOCKED_DM_PERMISSIONS
        } else {
            DM_PERMISSIONS
        };
        return Ok((ChannelContext { community_id: None, member: None }, perms));
    };

    let ctx = member_context(db, user_id, community_id).await?;
    let overrides: Vec<(Uuid, i64, i64)> =
        sqlx::query_as("SELECT role_id, allow, deny FROM channel_overrides WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_all(db)
            .await?;
    let overrides: Vec<Override> = overrides
        .into_iter()
        .map(|(role_id, allow, deny)| Override {
            role_id,
            allow: Permissions(allow),
            deny: Permissions(deny),
        })
        .collect();

    let perms = ctx.in_channel(&overrides);
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Err(ApiError::not_found());
    }
    Ok((ChannelContext { community_id: Some(community_id), member: Some(ctx) }, perms))
}

/// IDs aller Kanäle der Community, die der Nutzer sehen darf (`VIEW_CHANNEL` inkl. Overrides).
pub async fn visible_channel_ids(db: &PgPool, user_id: Uuid, community_id: Uuid) -> ApiResult<Vec<Uuid>> {
    let ctx = member_context(db, user_id, community_id).await?;
    let overrides = community_overrides(db, community_id).await?;
    let channels: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM channels WHERE community_id = $1")
        .bind(community_id)
        .fetch_all(db)
        .await?;
    Ok(channels
        .into_iter()
        .filter(|id| {
            ctx.in_channel(overrides.get(id).map_or(&[], Vec::as_slice)).contains(Permissions::VIEW_CHANNEL)
        })
        .collect())
}

/// Discord-Rechtebits (Werte aus der Discord-API-Doku), soweit Lykos eine Entsprechung hat.
pub mod discord {
    pub const CREATE_INSTANT_INVITE: u64 = 1 << 0;
    pub const KICK_MEMBERS: u64 = 1 << 1;
    pub const BAN_MEMBERS: u64 = 1 << 2;
    pub const ADMINISTRATOR: u64 = 1 << 3;
    pub const MANAGE_CHANNELS: u64 = 1 << 4;
    pub const MANAGE_GUILD: u64 = 1 << 5;
    pub const ADD_REACTIONS: u64 = 1 << 6;
    pub const VIEW_AUDIT_LOG: u64 = 1 << 7;
    pub const STREAM: u64 = 1 << 9;
    pub const VIEW_CHANNEL: u64 = 1 << 10;
    pub const SEND_MESSAGES: u64 = 1 << 11;
    pub const MANAGE_MESSAGES: u64 = 1 << 13;
    pub const ATTACH_FILES: u64 = 1 << 15;
    pub const MENTION_EVERYONE: u64 = 1 << 17;
    pub const CONNECT: u64 = 1 << 20;
    pub const SPEAK: u64 = 1 << 21;
    pub const CHANGE_NICKNAME: u64 = 1 << 26;
    pub const MANAGE_NICKNAMES: u64 = 1 << 27;
    pub const MANAGE_ROLES: u64 = 1 << 28;
    pub const MANAGE_WEBHOOKS: u64 = 1 << 29;
    pub const MANAGE_GUILD_EXPRESSIONS: u64 = 1 << 30;
}

/// Zuordnung Discord-Bit → Lykos-Recht. Alles, was hier fehlt, wird beim Import verworfen.
const DISCORD_MAP: &[(u64, Permissions)] = &[
    (discord::CREATE_INSTANT_INVITE, Permissions::CREATE_INVITES),
    (discord::KICK_MEMBERS, Permissions::KICK_MEMBERS),
    (discord::BAN_MEMBERS, Permissions::BAN_MEMBERS),
    (discord::ADMINISTRATOR, Permissions::ADMINISTRATOR),
    (discord::MANAGE_CHANNELS, Permissions::MANAGE_CHANNELS),
    (discord::MANAGE_GUILD, Permissions::MANAGE_COMMUNITY),
    (discord::ADD_REACTIONS, Permissions::ADD_REACTIONS),
    (discord::VIEW_AUDIT_LOG, Permissions::VIEW_AUDIT_LOG),
    // Discord fasst Bildschirm und Kamera unter „Video“ (STREAM) zusammen, Lykos trennt beides.
    (discord::STREAM, Permissions(Permissions::STREAM.0 | Permissions::VIDEO.0)),
    (discord::VIEW_CHANNEL, Permissions::VIEW_CHANNEL),
    (discord::SEND_MESSAGES, Permissions::SEND_MESSAGES),
    (discord::MANAGE_MESSAGES, Permissions::MANAGE_MESSAGES),
    (discord::ATTACH_FILES, Permissions::ATTACH_FILES),
    (discord::MENTION_EVERYONE, Permissions::MENTION_EVERYONE),
    (discord::CONNECT, Permissions::CONNECT),
    (discord::SPEAK, Permissions::SPEAK),
    (discord::CHANGE_NICKNAME, Permissions::CHANGE_NICKNAME),
    (discord::MANAGE_NICKNAMES, Permissions::MANAGE_NICKNAMES),
    (discord::MANAGE_ROLES, Permissions::MANAGE_ROLES),
    (discord::MANAGE_WEBHOOKS, Permissions::MANAGE_WEBHOOKS),
    (discord::MANAGE_GUILD_EXPRESSIONS, Permissions::MANAGE_EMOJIS),
];

/// Discord-Rechte (Bitfeld als `u64`, Discord liefert Dezimal-Strings, die über `i64` hinausgehen können)
/// in Lykos-Rechte übersetzen. Unbekannte bzw. in Lykos nicht vorhandene Bits fallen weg.
pub fn from_discord(bits: u64) -> Permissions {
    DISCORD_MAP
        .iter()
        .filter(|(discord, _)| bits & discord != 0)
        .fold(Permissions::NONE, |acc, (_, lykos)| acc | *lykos)
        .sanitized()
}

/// Bits aus `bits`, die `from_discord` verwirft (für Hinweise beim Import).
pub fn discord_unmapped(bits: u64) -> u64 {
    let known = DISCORD_MAP.iter().fold(0, |acc, (discord, _)| acc | discord);
    bits & !known
}

/// Gibt `Forbidden` zurück, wenn `have` das Recht `need` nicht enthält.
pub fn require(have: Permissions, need: Permissions) -> ApiResult<()> {
    if have.contains(need) { Ok(()) } else { Err(ApiError::forbidden()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(roles: Vec<(Uuid, Permissions)>, default_role_id: Uuid) -> MemberContext {
        MemberContext { community_id: Uuid::nil(), is_owner: false, default_role_id, roles }
    }

    #[test]
    fn owner_has_everything() {
        let d = Uuid::now_v7();
        let mut c = ctx(vec![(d, Permissions::NONE)], d);
        c.is_owner = true;
        assert_eq!(c.base(), Permissions::ALL);
        let o = Override { role_id: d, allow: Permissions::NONE, deny: Permissions::ALL };
        assert_eq!(c.in_channel(&[o]), Permissions::ALL);
    }

    #[test]
    fn roles_are_ored() {
        let d = Uuid::now_v7();
        let mods = Uuid::now_v7();
        let c = ctx(vec![(d, Permissions::DEFAULT_ROLE), (mods, Permissions::MANAGE_MESSAGES)], d);
        assert!(c.base().contains(Permissions::SEND_MESSAGES | Permissions::MANAGE_MESSAGES));
        assert!(!c.base().contains(Permissions::MANAGE_CHANNELS));
    }

    #[test]
    fn administrator_implies_all() {
        let d = Uuid::now_v7();
        let admin = Uuid::now_v7();
        let c = ctx(vec![(d, Permissions::NONE), (admin, Permissions::ADMINISTRATOR)], d);
        assert_eq!(c.base(), Permissions::ALL);
    }

    #[test]
    fn default_override_then_role_override() {
        let d = Uuid::now_v7();
        let team = Uuid::now_v7();
        let c = ctx(vec![(d, Permissions::DEFAULT_ROLE), (team, Permissions::NONE)], d);

        // Privater Kanal: Standardrolle darf nicht sehen, Team-Rolle schon.
        let overrides = [
            Override { role_id: d, allow: Permissions::NONE, deny: Permissions::VIEW_CHANNEL },
            Override { role_id: team, allow: Permissions::VIEW_CHANNEL, deny: Permissions::NONE },
        ];
        assert!(c.in_channel(&overrides).contains(Permissions::VIEW_CHANNEL));

        // Ohne Team-Rolle: kein Zugriff.
        let outsider = ctx(vec![(d, Permissions::DEFAULT_ROLE)], d);
        assert!(!outsider.in_channel(&overrides).contains(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn role_allow_beats_role_deny_of_other_role() {
        // Wie bei Discord: Rollen-Overrides werden gesammelt, allow gewinnt gegen deny.
        let d = Uuid::now_v7();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let c = ctx(vec![(d, Permissions::DEFAULT_ROLE), (a, Permissions::NONE), (b, Permissions::NONE)], d);
        let overrides = [
            Override { role_id: a, allow: Permissions::NONE, deny: Permissions::SEND_MESSAGES },
            Override { role_id: b, allow: Permissions::SEND_MESSAGES, deny: Permissions::NONE },
        ];
        assert!(c.in_channel(&overrides).contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn dm_permissions_are_limited() {
        assert!(DM_PERMISSIONS.contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES));
        assert!(DM_PERMISSIONS.contains(Permissions::ATTACH_FILES | Permissions::ADD_REACTIONS));
        assert!(!DM_PERMISSIONS.contains(Permissions::MENTION_EVERYONE));
        assert!(!DM_PERMISSIONS.contains(Permissions::MANAGE_MESSAGES));
        assert!(!DM_PERMISSIONS.contains(Permissions::MANAGE_CHANNELS));
        assert!(!DM_PERMISSIONS.contains(Permissions::MANAGE_ROLES));
    }

    #[test]
    fn sanitize_drops_unknown_bits() {
        assert_eq!(Permissions(1 << 40 | 1).sanitized(), Permissions::VIEW_CHANNEL);
    }

    #[test]
    fn video_is_known_and_default() {
        assert!(Permissions::ALL.contains(Permissions::VIDEO));
        assert!(Permissions::DEFAULT_ROLE.contains(Permissions::VIDEO));
        assert_eq!(Permissions::VIDEO.sanitized(), Permissions::VIDEO);
    }

    #[test]
    fn message_feature_bits() {
        assert!(Permissions::ALL.contains(Permissions::ADD_REACTIONS | Permissions::MENTION_EVERYONE));
        assert!(Permissions::DEFAULT_ROLE.contains(Permissions::ADD_REACTIONS));
        assert!(!Permissions::DEFAULT_ROLE.contains(Permissions::MENTION_EVERYONE));
    }

    #[test]
    fn nickname_permissions() {
        assert!(Permissions::ALL.contains(Permissions::CHANGE_NICKNAME | Permissions::MANAGE_NICKNAMES));
        assert!(Permissions::DEFAULT_ROLE.contains(Permissions::CHANGE_NICKNAME));
        assert!(!Permissions::DEFAULT_ROLE.contains(Permissions::MANAGE_NICKNAMES));
        assert_eq!(Permissions::MANAGE_NICKNAMES.sanitized(), Permissions::MANAGE_NICKNAMES);
    }

    #[test]
    fn ban_members_is_known_but_not_default() {
        assert!(Permissions::ALL.contains(Permissions::BAN_MEMBERS));
        assert!(!Permissions::DEFAULT_ROLE.contains(Permissions::BAN_MEMBERS));
        assert_eq!(Permissions::BAN_MEMBERS.sanitized(), Permissions::BAN_MEMBERS);
    }

    #[test]
    fn discord_mapping() {
        use discord as d;
        assert_eq!(from_discord(0), Permissions::NONE);
        assert_eq!(from_discord(d::CREATE_INSTANT_INVITE), Permissions::CREATE_INVITES);
        assert_eq!(from_discord(d::KICK_MEMBERS), Permissions::KICK_MEMBERS);
        assert_eq!(from_discord(d::ADMINISTRATOR), Permissions::ADMINISTRATOR);
        assert_eq!(from_discord(d::MANAGE_CHANNELS), Permissions::MANAGE_CHANNELS);
        assert_eq!(from_discord(d::MANAGE_GUILD), Permissions::MANAGE_COMMUNITY);
        assert_eq!(from_discord(d::ADD_REACTIONS), Permissions::ADD_REACTIONS);
        assert_eq!(from_discord(d::VIEW_AUDIT_LOG), Permissions::VIEW_AUDIT_LOG);
        assert_eq!(from_discord(d::STREAM), Permissions::STREAM | Permissions::VIDEO);
        assert_eq!(from_discord(d::VIEW_CHANNEL), Permissions::VIEW_CHANNEL);
        assert_eq!(from_discord(d::SEND_MESSAGES), Permissions::SEND_MESSAGES);
        assert_eq!(from_discord(d::MANAGE_MESSAGES), Permissions::MANAGE_MESSAGES);
        assert_eq!(from_discord(d::ATTACH_FILES), Permissions::ATTACH_FILES);
        assert_eq!(from_discord(d::MENTION_EVERYONE), Permissions::MENTION_EVERYONE);
        assert_eq!(from_discord(d::CONNECT), Permissions::CONNECT);
        assert_eq!(from_discord(d::SPEAK), Permissions::SPEAK);
        assert_eq!(from_discord(d::CHANGE_NICKNAME), Permissions::CHANGE_NICKNAME);
        assert_eq!(from_discord(d::MANAGE_NICKNAMES), Permissions::MANAGE_NICKNAMES);
        assert_eq!(from_discord(d::MANAGE_ROLES), Permissions::MANAGE_ROLES);
        assert_eq!(from_discord(d::MANAGE_GUILD_EXPRESSIONS), Permissions::MANAGE_EMOJIS);
        assert_eq!(from_discord(d::BAN_MEMBERS), Permissions::BAN_MEMBERS);
        assert_eq!(from_discord(d::MANAGE_WEBHOOKS), Permissions::MANAGE_WEBHOOKS);
        // Ohne Entsprechung (PRIORITY_SPEAKER = 1 << 8) und unbekannte Bits fallen weg.
        const PRIORITY_SPEAKER: u64 = 1 << 8;
        assert_eq!(from_discord(PRIORITY_SPEAKER | 1 << 40), Permissions::NONE);
        assert_eq!(
            discord_unmapped(PRIORITY_SPEAKER | d::VIEW_CHANNEL | d::MANAGE_WEBHOOKS),
            PRIORITY_SPEAKER
        );
        // Werte über i64::MAX bleiben gültig.
        assert_eq!(from_discord(u64::MAX), from_discord(u64::MAX).sanitized());
        assert!(from_discord(u64::MAX).contains(Permissions::ADMINISTRATOR | Permissions::VIEW_CHANNEL));
        // Typische @everyone-Rechte aus der Fixture.
        let everyone = from_discord(2_222_085_186_768_449);
        assert!(
            everyone.contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::CONNECT)
        );
        assert!(!everyone.contains(Permissions::ADMINISTRATOR));
    }

    #[test]
    fn manage_webhooks_is_known_but_not_default() {
        assert!(Permissions::ALL.contains(Permissions::MANAGE_WEBHOOKS));
        assert!(!Permissions::DEFAULT_ROLE.contains(Permissions::MANAGE_WEBHOOKS));
        assert_eq!(Permissions::MANAGE_WEBHOOKS.sanitized(), Permissions::MANAGE_WEBHOOKS);
    }

    #[test]
    fn manage_emojis_is_known_but_not_default() {
        assert!(Permissions::ALL.contains(Permissions::MANAGE_EMOJIS));
        assert!(!Permissions::DEFAULT_ROLE.contains(Permissions::MANAGE_EMOJIS));
        assert_eq!(Permissions::MANAGE_EMOJIS.sanitized(), Permissions::MANAGE_EMOJIS);
    }
}
