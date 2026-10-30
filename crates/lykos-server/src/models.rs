//! Datenobjekte der API (siehe `docs/api.md`) und die Abfragen, die sie laden.

use serde::{Deserialize, Deserializer, Serialize};
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{attachments::Attachment, embeds::Embed, error::ApiResult, permissions::Permissions};

/// Eigenes Konto (`/auth/me`, `READY`).
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub is_instance_admin: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Download-Pfad des Profilbilds (`/api/users/{id}/avatar/{avatar_id}`), sonst `null`.
    pub avatar_url: Option<String>,
    /// „Über mich“, max. 190 Zeichen.
    pub bio: Option<String>,
    /// Frei wählbarer Status-Text, max. 128 Zeichen.
    pub status_text: Option<String>,
    /// Pronomen, max. 40 Zeichen.
    pub pronouns: Option<String>,
    /// Download-Pfad des Profilbanners (`/api/users/{id}/banner/{banner_id}`), sonst `null`.
    pub banner_url: Option<String>,
    /// Bannerfarbe (0xRRGGBB), wenn kein Bannerbild gesetzt ist.
    pub banner_color: Option<i32>,
    /// Akzentfarbe der Profilkarte (0xRRGGBB).
    pub accent_color: Option<i32>,
    /// Bot-Konto (siehe `routes::bots`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
}

/// Öffentliche Kurzform eines Nutzers (Autor, Mitglied, Audit-Akteur).
#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Bot-Konto; fehlt bei Menschen. Gesetzt bei Mitgliedern und Nachrichten-Autoren.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
}

impl UserPublic {
    pub fn new(id: Uuid, username: String, display_name: Option<String>, avatar_id: Option<Uuid>) -> Self {
        Self { id, username, display_name, avatar_url: avatar_url(id, avatar_id), bot: false }
    }

    /// Markiert den Nutzer als Bot.
    pub fn with_bot(mut self, bot: bool) -> Self {
        self.bot = bot;
        self
    }
}

/// Profil eines Nutzers (`GET /users/{id}`, Gateway `USER_UPDATE`).
#[derive(Debug, Clone, Serialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub status_text: Option<String>,
    pub pronouns: Option<String>,
    pub banner_url: Option<String>,
    pub banner_color: Option<i32>,
    pub accent_color: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
}

impl From<User> for UserProfile {
    fn from(u: User) -> Self {
        UserProfile {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            avatar_url: u.avatar_url,
            bio: u.bio,
            status_text: u.status_text,
            pronouns: u.pronouns,
            banner_url: u.banner_url,
            banner_color: u.banner_color,
            accent_color: u.accent_color,
            created_at: u.created_at,
            bot: u.bot,
        }
    }
}

/// Download-Pfad eines Avatars. Die Avatar-ID wechselt bei jedem Upload, daher ist die URL cachebar.
pub fn avatar_url(user_id: Uuid, avatar_id: Option<Uuid>) -> Option<String> {
    avatar_id.map(|a| format!("/api/users/{user_id}/avatar/{a}"))
}

/// Download-Pfad eines Profilbanners (wechselt wie die Avatar-URL bei jedem Upload).
pub fn user_banner_url(user_id: Uuid, banner_id: Option<Uuid>) -> Option<String> {
    banner_id.map(|b| format!("/api/users/{user_id}/banner/{b}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct Community {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    pub message_retention_days: Option<i32>,
    /// Speicher-Kontingent der Community für Anhänge in MB; `None` = unbegrenzt.
    pub upload_quota_mb: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// `/api/communities/{id}/icon/{icon_id}` (nur für Mitglieder), sonst `null`.
    pub icon_url: Option<String>,
    /// `/api/communities/{id}/banner/{banner_id}` (nur für Mitglieder), sonst `null`.
    pub banner_url: Option<String>,
    /// Kurzbeschreibung, max. 300 Zeichen.
    pub description: Option<String>,
    /// Akzentfarbe (0xRRGGBB), dezent genutzt (Banner-Verlauf, Rail-Indikator).
    pub accent_color: Option<i32>,
    /// Textkanal für Beitritts-Meldungen; `null` = keine.
    pub system_channel_id: Option<Uuid>,
    /// Beitrittsmeldungen im Systemkanal senden.
    pub join_messages_enabled: bool,
    /// Eigener Willkommenstext (Platzhalter `{user}`, `{community}`); `null` = zufällige Standardsätze.
    pub welcome_message: Option<String>,
    /// Sprachkanal für Inaktive (dort weder Sprechen noch Stream/Kamera); `null` = keiner.
    pub afk_channel_id: Option<Uuid>,
    /// Sekunden ohne Aktivität, nach denen der Client in den AFK-Kanal wechselt.
    pub afk_timeout_seconds: i32,
}

/// Spalten für `CommunityRow` (Tabellen-Alias `c`).
#[macro_export]
macro_rules! community_columns {
    () => {
        "c.id, c.name, c.owner_id, c.message_retention_days, c.upload_quota_mb, c.created_at,
         c.icon_id, c.banner_id, c.description, c.accent_color, c.system_channel_id,
         c.join_messages_enabled, c.welcome_message, c.afk_channel_id, c.afk_timeout_seconds"
    };
}

#[derive(FromRow)]
pub struct CommunityRow {
    id: Uuid,
    name: String,
    owner_id: Uuid,
    message_retention_days: Option<i32>,
    upload_quota_mb: Option<i32>,
    created_at: OffsetDateTime,
    icon_id: Option<Uuid>,
    banner_id: Option<Uuid>,
    description: Option<String>,
    accent_color: Option<i32>,
    system_channel_id: Option<Uuid>,
    join_messages_enabled: bool,
    welcome_message: Option<String>,
    afk_channel_id: Option<Uuid>,
    afk_timeout_seconds: i32,
}

impl From<CommunityRow> for Community {
    fn from(r: CommunityRow) -> Self {
        Community {
            icon_url: r.icon_id.map(|i| format!("/api/communities/{}/icon/{i}", r.id)),
            banner_url: r.banner_id.map(|b| format!("/api/communities/{}/banner/{b}", r.id)),
            id: r.id,
            name: r.name,
            owner_id: r.owner_id,
            message_retention_days: r.message_retention_days,
            upload_quota_mb: r.upload_quota_mb,
            created_at: r.created_at,
            description: r.description,
            accent_color: r.accent_color,
            system_channel_id: r.system_channel_id,
            join_messages_enabled: r.join_messages_enabled,
            welcome_message: r.welcome_message,
            afk_channel_id: r.afk_channel_id,
            afk_timeout_seconds: r.afk_timeout_seconds,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Channel {
    pub id: Uuid,
    /// `None` bei DM-Kanälen (`kind` = `dm` | `group_dm`).
    pub community_id: Option<Uuid>,
    pub name: String,
    pub kind: String,
    pub topic: Option<String>,
    pub position: i32,
    /// Ton-Einstellungen (nur bei Sprachkanälen wirksam): Ziel-Bitrate der Mikrofonspur in kbit/s (32–256).
    pub audio_bitrate: i32,
    /// Opus in Stereo veröffentlichen.
    pub audio_stereo: bool,
    /// Discontinuous Transmission (Sprechpausen kaum übertragen).
    pub audio_dtx: bool,
    /// Redundant Audio Data (robuster bei Paketverlust).
    pub audio_red: bool,
    /// Höchstzahl gleichzeitiger Teilnehmer (1–99), `None` = unbegrenzt.
    pub user_limit: Option<i32>,
    /// Kategorie in der Seitenleiste; `None` = ohne Kategorie (oben) bzw. bei DM-Kanälen.
    pub category_id: Option<Uuid>,
}

/// Spalten für `Channel` (ohne Tabellen-Alias).
#[macro_export]
macro_rules! channel_columns {
    () => {
        "id, community_id, name, kind, topic, position, audio_bitrate, audio_stereo, audio_dtx, audio_red, user_limit,
         category_id"
    };
}

/// Kanal-Kategorie einer Community (nur Gruppierung, ohne eigene Rechte).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Category {
    pub id: Uuid,
    pub community_id: Uuid,
    pub name: String,
    pub position: i32,
}

/// Spalten für `Category` (ohne Tabellen-Alias).
#[macro_export]
macro_rules! category_columns {
    () => {
        "id, community_id, name, position"
    };
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Role {
    pub id: Uuid,
    pub community_id: Uuid,
    pub name: String,
    pub permissions: Permissions,
    pub position: i32,
    pub color: Option<i32>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub user: UserPublic,
    pub nickname: Option<String>,
    pub role_ids: Vec<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    /// `None` bei Nachrichten in DM-Kanälen.
    pub community_id: Option<Uuid>,
    pub author: Option<UserPublic>,
    pub content: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub edited_at: Option<OffsetDateTime>,
    pub attachments: Vec<Attachment>,
    /// Nachricht, auf die geantwortet wird (bleibt gesetzt, auch wenn das Original gelöscht ist).
    pub reply_to_id: Option<Uuid>,
    /// Kompakte Vorschau des Originals; `null`, wenn es gelöscht wurde (oder keine Antwort).
    pub referenced_message: Option<MessageReference>,
    /// Aggregierte Reaktionen in Reihenfolge der ersten Reaktion.
    pub reactions: Vec<Reaction>,
    /// IDs erwähnter Nutzer.
    pub mentions: Vec<Uuid>,
    pub mention_everyone: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub pinned_at: Option<OffsetDateTime>,
    /// `default` oder Systemnachricht mit leerem `content`: `member_join` (`author` ist beigetreten),
    /// `call_missed` (`author` hat angerufen), `recipient_add` (`author` hat `mentions` zur Gruppe hinzugefügt).
    pub kind: String,
    /// Webhook, der die Nachricht gesendet hat (`author` ist dann `null`); `null`, wenn der Webhook
    /// inzwischen gelöscht ist (`webhook` bleibt mit dem Namen erhalten).
    pub webhook_id: Option<Uuid>,
    /// Anzeige eines Webhook-Absenders; `null` bei normalen Nachrichten.
    pub webhook: Option<WebhookAuthor>,
    /// Bereinigte Embeds (Karten, ohne Bilder; siehe `embeds`).
    pub embeds: Vec<Embed>,
}

/// Absender-Anzeige einer Webhook-Nachricht.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookAuthor {
    /// Anzeigename (aus `username` der Anfrage oder Name des Webhooks).
    pub name: String,
    /// Avatar des Lykos-Webhooks (nie eine fremde `avatar_url`), sonst `null`.
    pub avatar_url: Option<String>,
}

/// Download-Pfad des Webhook-Avatars (wechselt bei jedem Upload).
pub fn webhook_avatar_url(webhook_id: Uuid, avatar_id: Option<Uuid>) -> Option<String> {
    avatar_id.map(|a| format!("/api/webhooks/{webhook_id}/avatar/{a}"))
}

/// Vorschau der Nachricht, auf die geantwortet wird.
#[derive(Debug, Clone, Serialize)]
pub struct MessageReference {
    pub id: Uuid,
    pub author: Option<UserPublic>,
    /// Name des Webhooks, wenn das Original von einem Webhook stammt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_name: Option<String>,
    /// Auf 200 Zeichen gekürzt.
    pub content: String,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    /// Unicode-Emoji oder bei Custom-Emotes `name:id` (aktueller Name, bei gelöschten der letzte bekannte).
    pub emoji: String,
    pub count: i64,
    /// Hat der abfragende Nutzer so reagiert? In Gateway-Events immer `false`.
    pub me: bool,
    /// ID des Custom-Emotes, sonst `null`.
    #[serde(default)]
    pub emoji_id: Option<Uuid>,
    /// Animiertes Custom-Emote (GIF)?
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Invite {
    pub code: String,
    pub community_id: Uuid,
    pub max_uses: Option<i32>,
    pub uses: i32,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl sqlx::Type<sqlx::Postgres> for Permissions {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <i64 as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Permissions {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        Ok(Permissions(<i64 as sqlx::Decode<sqlx::Postgres>>::decode(value)?))
    }
}

/// Kanalarten mit Nachrichten: Textkanäle und DMs.
pub fn is_text_like(kind: &str) -> bool {
    matches!(kind, "text" | "dm" | "group_dm")
}

/// DM-Kanalarten (ohne Community).
pub fn is_dm_kind(kind: &str) -> bool {
    matches!(kind, "dm" | "group_dm")
}

// --- Abfragen ---------------------------------------------------------------

#[derive(FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    display_name: Option<String>,
    is_instance_admin: bool,
    created_at: OffsetDateTime,
    avatar_id: Option<Uuid>,
    bio: Option<String>,
    status_text: Option<String>,
    pronouns: Option<String>,
    banner_id: Option<Uuid>,
    banner_color: Option<i32>,
    accent_color: Option<i32>,
    is_bot: bool,
}

pub async fn user_by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, username, display_name, is_instance_admin, created_at, avatar_id, bio, status_text,
                pronouns, banner_id, banner_color, accent_color, is_bot
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| User {
        avatar_url: avatar_url(r.id, r.avatar_id),
        banner_url: user_banner_url(r.id, r.banner_id),
        id: r.id,
        username: r.username,
        display_name: r.display_name,
        is_instance_admin: r.is_instance_admin,
        created_at: r.created_at,
        bio: r.bio,
        status_text: r.status_text,
        pronouns: r.pronouns,
        banner_color: r.banner_color,
        accent_color: r.accent_color,
        bot: r.is_bot,
    }))
}

/// Darf `viewer` das Profil (und den Avatar) von `target` sehen? Ja für sich selbst, bei
/// mindestens einer gemeinsamen Community, bei einem gemeinsamen DM-Kanal, wenn beide befreundet sind und
/// wenn `target` an `viewer` eine (noch offene) Freundschaftsanfrage geschickt hat. Eine eigene Anfrage oder
/// eine eigene Blockade öffnet das Profil eines Fremden nicht – die Freundesliste zeigt dafür nur Name und
/// Avatar-URL (`UserPublic`).
pub async fn can_see_user(db: impl PgExecutor<'_>, viewer: Uuid, target: Uuid) -> ApiResult<bool> {
    if viewer == target {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM members a JOIN members b ON a.community_id = b.community_id
                        WHERE a.user_id = $1 AND b.user_id = $2)
             OR EXISTS (SELECT 1 FROM dm_recipients a JOIN dm_recipients b ON a.channel_id = b.channel_id
                        WHERE a.user_id = $1 AND b.user_id = $2)
             OR EXISTS (SELECT 1 FROM relationships WHERE user_id = $1 AND target_id = $2
                        AND kind IN ('friend', 'pending_in'))",
    )
    .bind(viewer)
    .bind(target)
    .fetch_one(db)
    .await?)
}

pub async fn community_by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<Community>> {
    let row: Option<CommunityRow> =
        sqlx::query_as(concat!("SELECT ", community_columns!(), " FROM communities c WHERE c.id = $1"))
            .bind(id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(Community::from))
}

pub async fn channels_of(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Vec<Channel>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        channel_columns!(),
        " FROM channels WHERE community_id = $1 ORDER BY kind DESC, position, id"
    ))
    .bind(community_id)
    .fetch_all(db)
    .await?)
}

pub async fn categories_of(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Vec<Category>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        category_columns!(),
        " FROM channel_categories WHERE community_id = $1 ORDER BY position, id"
    ))
    .bind(community_id)
    .fetch_all(db)
    .await?)
}

pub async fn category_by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<Category>> {
    Ok(sqlx::query_as(concat!("SELECT ", category_columns!(), " FROM channel_categories WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?)
}

pub async fn channel_by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<Channel>> {
    Ok(sqlx::query_as(concat!("SELECT ", channel_columns!(), " FROM channels WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?)
}

pub async fn roles_of(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Vec<Role>> {
    Ok(sqlx::query_as(
        "SELECT id, community_id, name, permissions, position, color, is_default FROM roles
         WHERE community_id = $1 ORDER BY is_default DESC, position, id",
    )
    .bind(community_id)
    .fetch_all(db)
    .await?)
}

pub async fn role_by_id(db: impl PgExecutor<'_>, id: Uuid) -> ApiResult<Option<Role>> {
    Ok(sqlx::query_as(
        "SELECT id, community_id, name, permissions, position, color, is_default FROM roles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

#[derive(FromRow)]
struct MemberRow {
    user_id: Uuid,
    username: String,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    nickname: Option<String>,
    joined_at: OffsetDateTime,
    role_ids: Vec<Uuid>,
    is_bot: bool,
}

impl From<MemberRow> for Member {
    fn from(r: MemberRow) -> Self {
        Member {
            user: UserPublic::new(r.user_id, r.username, r.display_name, r.avatar_id).with_bot(r.is_bot),
            nickname: r.nickname,
            role_ids: r.role_ids,
            joined_at: r.joined_at,
        }
    }
}

macro_rules! member_select {
    () => {
        "SELECT m.user_id, u.username, u.display_name, u.avatar_id, m.nickname, m.joined_at, u.is_bot,
        COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS role_ids
     FROM members m
     JOIN users u ON u.id = m.user_id
     LEFT JOIN member_roles mr ON mr.community_id = m.community_id AND mr.user_id = m.user_id"
    };
}

pub async fn members_of(db: impl PgExecutor<'_>, community_id: Uuid) -> ApiResult<Vec<Member>> {
    let rows: Vec<MemberRow> = sqlx::query_as(concat!(
        member_select!(),
        " WHERE m.community_id = $1
         GROUP BY m.user_id, u.username, u.display_name, u.avatar_id, m.nickname, m.joined_at, u.is_bot
         ORDER BY lower(u.username)"
    ))
    .bind(community_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Member::from).collect())
}

pub async fn member(db: impl PgExecutor<'_>, community_id: Uuid, user_id: Uuid) -> ApiResult<Option<Member>> {
    let row: Option<MemberRow> = sqlx::query_as(concat!(
        member_select!(),
        " WHERE m.community_id = $1 AND m.user_id = $2
         GROUP BY m.user_id, u.username, u.display_name, u.avatar_id, m.nickname, m.joined_at, u.is_bot"
    ))
    .bind(community_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(Member::from))
}

#[derive(FromRow)]
struct MembershipRow {
    community_id: Uuid,
    #[sqlx(flatten)]
    member: MemberRow,
}

/// Alle Mitgliedschaften eines Nutzers als `(community_id, Member)` – eine Abfrage statt einer je Community.
pub async fn memberships_of(db: impl PgExecutor<'_>, user_id: Uuid) -> ApiResult<Vec<(Uuid, Member)>> {
    // Wie `member_select!`, zusätzlich mit `community_id`.
    let rows: Vec<MembershipRow> = sqlx::query_as(
        "SELECT m.community_id, m.user_id, u.username, u.display_name, u.avatar_id, m.nickname, m.joined_at, u.is_bot,
            COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS role_ids
         FROM members m
         JOIN users u ON u.id = m.user_id
         LEFT JOIN member_roles mr ON mr.community_id = m.community_id AND mr.user_id = m.user_id
         WHERE m.user_id = $1
         GROUP BY m.community_id, m.user_id, u.username, u.display_name, u.avatar_id, m.nickname, m.joined_at, u.is_bot",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.community_id, Member::from(r.member))).collect())
}

#[derive(FromRow)]
struct MessageRow {
    id: Uuid,
    channel_id: Uuid,
    community_id: Option<Uuid>,
    author_id: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
    content: String,
    created_at: OffsetDateTime,
    edited_at: Option<OffsetDateTime>,
    attachments: sqlx::types::Json<Vec<Attachment>>,
    reply_to_id: Option<Uuid>,
    ref_id: Option<Uuid>,
    ref_author_id: Option<Uuid>,
    ref_username: Option<String>,
    ref_display_name: Option<String>,
    ref_avatar_id: Option<Uuid>,
    ref_content: Option<String>,
    ref_has_attachments: Option<bool>,
    reactions: sqlx::types::Json<Vec<Reaction>>,
    mentions: Vec<Uuid>,
    mention_everyone: bool,
    pinned_at: Option<OffsetDateTime>,
    kind: String,
    author_is_bot: Option<bool>,
    ref_webhook_name: Option<String>,
    webhook_id: Option<Uuid>,
    webhook_name: Option<String>,
    webhook_avatar_id: Option<Uuid>,
    embeds: sqlx::types::Json<Vec<Embed>>,
}

fn user_public(
    id: Option<Uuid>,
    username: Option<String>,
    display_name: Option<String>,
    avatar_id: Option<Uuid>,
) -> Option<UserPublic> {
    match (id, username) {
        (Some(id), Some(username)) => Some(UserPublic::new(id, username, display_name, avatar_id)),
        _ => None,
    }
}

impl From<MessageRow> for Message {
    fn from(r: MessageRow) -> Self {
        let referenced_message = r.ref_id.map(|id| MessageReference {
            id,
            author: user_public(r.ref_author_id, r.ref_username, r.ref_display_name, r.ref_avatar_id),
            webhook_name: r.ref_webhook_name,
            content: r.ref_content.unwrap_or_default(),
            has_attachments: r.ref_has_attachments.unwrap_or(false),
        });
        let webhook = r.webhook_name.map(|name| WebhookAuthor {
            name,
            avatar_url: r.webhook_id.and_then(|id| webhook_avatar_url(id, r.webhook_avatar_id)),
        });
        Message {
            id: r.id,
            channel_id: r.channel_id,
            community_id: r.community_id,
            author: user_public(r.author_id, r.username, r.display_name, r.avatar_id)
                .map(|a| a.with_bot(r.author_is_bot.unwrap_or(false))),
            webhook_id: r.webhook_id,
            webhook,
            embeds: r.embeds.0,
            content: r.content,
            created_at: r.created_at,
            edited_at: r.edited_at,
            attachments: r.attachments.0.into_iter().map(Attachment::with_url).collect(),
            reply_to_id: r.reply_to_id,
            referenced_message,
            reactions: r.reactions.0,
            mentions: r.mentions,
            mention_everyone: r.mention_everyone,
            pinned_at: r.pinned_at,
            kind: r.kind,
        }
    }
}

/// `SELECT` für `Message`. Das Argument ist der Platzhalter der Nutzer-ID, für die
/// `reactions[].me` berechnet wird (`NULL` → immer `false`, z. B. für Gateway-Events).
macro_rules! message_select {
    ($viewer:literal) => {
        concat!(
            "SELECT msg.id, msg.channel_id, c.community_id, msg.author_id,
        u.username, u.display_name, u.avatar_id, msg.content, msg.created_at, msg.edited_at, ",
            crate::attachment_json!(),
            " AS attachments,
        msg.reply_to_id, ref.id AS ref_id, ref.author_id AS ref_author_id,
        ru.username AS ref_username, ru.display_name AS ref_display_name, ru.avatar_id AS ref_avatar_id,
        left(ref.content, 200) AS ref_content,
        EXISTS (SELECT 1 FROM attachments ra WHERE ra.message_id = ref.id) AS ref_has_attachments,
        COALESCE((SELECT json_agg(json_build_object(
                    'emoji', CASE WHEN r.emoji_id IS NULL THEN r.emoji
                                  ELSE COALESCE(ce.name, r.emoji_name, 'emoji') || ':' || r.emoji END,
                    'count', r.count, 'me', r.me, 'emoji_id', r.emoji_id,
                    'animated', COALESCE(ce.animated, false))
                  ORDER BY r.first, r.emoji)
                  FROM (SELECT mr.emoji, count(*) AS count,
                               COALESCE(bool_or(mr.user_id = ",
            $viewer,
            "::uuid), false) AS me,
                               min(mr.created_at) AS first, max(mr.emoji_name) AS emoji_name,
                               CASE WHEN mr.emoji ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                                    THEN mr.emoji::uuid END AS emoji_id
                        FROM message_reactions mr WHERE mr.message_id = msg.id GROUP BY mr.emoji) r
                  LEFT JOIN custom_emojis ce ON ce.id = r.emoji_id),
                 '[]'::json) AS reactions,
        COALESCE((SELECT array_agg(mm.user_id ORDER BY mm.user_id) FROM message_mentions mm
                  WHERE mm.message_id = msg.id), '{}') AS mentions,
        msg.mention_everyone, msg.pinned_at, msg.kind, u.is_bot AS author_is_bot,
        ref.webhook_name AS ref_webhook_name, msg.webhook_id, msg.webhook_name,
        wh.avatar_id AS webhook_avatar_id, msg.embeds
     FROM messages msg
     JOIN channels c ON c.id = msg.channel_id
     LEFT JOIN users u ON u.id = msg.author_id
     LEFT JOIN webhooks wh ON wh.id = msg.webhook_id
     LEFT JOIN messages ref ON ref.id = msg.reply_to_id AND ref.channel_id = msg.channel_id
     LEFT JOIN users ru ON ru.id = ref.author_id"
        )
    };
}

/// Lädt eine Nachricht. `viewer` bestimmt `reactions[].me` (`None` für Broadcasts).
pub async fn message_by_id(
    db: impl PgExecutor<'_>,
    id: Uuid,
    viewer: Option<Uuid>,
) -> ApiResult<Option<Message>> {
    let row: Option<MessageRow> = sqlx::query_as(concat!(message_select!("$2"), " WHERE msg.id = $1"))
        .bind(id)
        .bind(viewer)
        .fetch_optional(db)
        .await?;
    Ok(row.map(Message::from))
}

/// Nachrichten eines Kanals, neueste zuerst, optional nur älter als `before`.
pub async fn messages_page(
    db: impl PgExecutor<'_>,
    channel_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
    viewer: Uuid,
) -> ApiResult<Vec<Message>> {
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$4"),
        " WHERE msg.channel_id = $1 AND ($2::uuid IS NULL OR msg.id < $2)
         ORDER BY msg.id DESC LIMIT $3"
    ))
    .bind(channel_id)
    .bind(before)
    .bind(limit)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Message::from).collect())
}

/// Angepinnte Nachrichten eines Kanals, zuletzt angepinnte zuerst.
pub async fn pinned_messages(
    db: impl PgExecutor<'_>,
    channel_id: Uuid,
    viewer: Uuid,
) -> ApiResult<Vec<Message>> {
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$2"),
        " WHERE msg.channel_id = $1 AND msg.pinned_at IS NOT NULL
         ORDER BY msg.pinned_at DESC, msg.id DESC"
    ))
    .bind(channel_id)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Message::from).collect())
}

/// Nachrichten um `around` herum, neueste zuerst: bis zu `limit / 2` neuere und den Rest aus
/// `around` selbst und älteren. Existiert `around` nicht (mehr), ist es einfach ein Zeitpunkt.
pub async fn messages_around(
    db: &sqlx::PgPool,
    channel_id: Uuid,
    around: Uuid,
    limit: i64,
    viewer: Uuid,
) -> ApiResult<Vec<Message>> {
    let mut newer: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$4"),
        " WHERE msg.channel_id = $1 AND msg.id > $2 ORDER BY msg.id ASC LIMIT $3"
    ))
    .bind(channel_id)
    .bind(around)
    .bind(limit / 2)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    let older: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$4"),
        " WHERE msg.channel_id = $1 AND msg.id <= $2 ORDER BY msg.id DESC LIMIT $3"
    ))
    .bind(channel_id)
    .bind(around)
    .bind(limit - newer.len() as i64)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    newer.reverse();
    Ok(newer.into_iter().chain(older).map(Message::from).collect())
}

/// Die ältesten `limit` Nachrichten neuer als `after`, neueste zuerst.
pub async fn messages_after(
    db: impl PgExecutor<'_>,
    channel_id: Uuid,
    after: Uuid,
    limit: i64,
    viewer: Uuid,
) -> ApiResult<Vec<Message>> {
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$4"),
        " WHERE msg.channel_id = $1 AND msg.id > $2 ORDER BY msg.id ASC LIMIT $3"
    ))
    .bind(channel_id)
    .bind(after)
    .bind(limit)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().rev().map(Message::from).collect())
}

/// Volltextsuche in den gegebenen Kanälen, neueste zuerst. `tsquery` stammt aus
/// `routes::search::build_tsquery`. Die `to_tsvector`-Ausdrücke entsprechen den GIN-Indizes aus 0010.
pub async fn search_messages(
    db: impl PgExecutor<'_>,
    channel_ids: &[Uuid],
    tsquery: &str,
    author_id: Option<Uuid>,
    before: Option<Uuid>,
    limit: i64,
    viewer: Uuid,
) -> ApiResult<Vec<Message>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!("$6"),
        " WHERE msg.channel_id = ANY($1)
           AND msg.kind <> 'automod_alert'
           AND (to_tsvector('german', msg.content) @@ to_tsquery('german', $2)
                OR to_tsvector('simple', msg.content) @@ to_tsquery('simple', $2))
           AND ($3::uuid IS NULL OR msg.author_id = $3)
           AND ($4::uuid IS NULL OR msg.id < $4)
         ORDER BY msg.id DESC LIMIT $5"
    ))
    .bind(channel_ids)
    .bind(tsquery)
    .bind(author_id)
    .bind(before)
    .bind(limit)
    .bind(viewer)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Message::from).collect())
}

// --- Hilfen für PATCH-Bodies --------------------------------------------------

/// Unterscheidet bei PATCH „Feld fehlt“ (`None`) von „Feld ist null“ (`Some(None)`).
pub fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}
