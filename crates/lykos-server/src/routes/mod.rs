use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post, put},
};

use crate::{gateway, state::AppState};

mod admin;
mod attachments;
pub mod auth;
mod automod;
pub mod bans;
pub mod bots;
pub mod calls;
pub mod categories;
pub mod channels;
pub mod communities;
pub mod customization;
mod desktop;
pub mod dms;
mod emojis;
pub mod feedback;
mod health;
mod instance;
pub mod invites;
mod legal;
pub(crate) mod messages;
mod oidc;
mod password_reset;
mod pins;
mod reactions;
pub mod read_states;
pub mod relationships;
pub mod roles;
mod search;
pub mod security;
mod usage;
pub mod users;
mod voice;
pub mod webhooks;

/// Alle API-Routen, gemountet unter `/api`.
pub fn api() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        .route("/instance", get(instance::instance))
        // Rechtstexte der Instanz (öffentlich)
        .route("/legal/imprint", get(legal::imprint))
        .route("/legal/privacy", get(legal::privacy))
        // Desktop-App: Download-Seite und Update-Manifest (öffentlich)
        .route("/desktop/latest", get(desktop::latest))
        .route("/desktop/latest.json", get(desktop::update_manifest))
        .route("/gateway", get(gateway::handler))
        // Auth
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me).patch(auth::update_me).delete(auth::delete_me))
        .route("/auth/me/export", get(auth::export))
        .route(
            "/auth/me/avatar",
            post(users::upload_avatar)
                .delete(users::delete_avatar)
                .layer(DefaultBodyLimit::max(users::AVATAR_MAX_BYTES + 64 * 1024)),
        )
        .route(
            "/auth/me/banner",
            post(customization::upload_user_banner).delete(customization::delete_user_banner).layer(
                DefaultBodyLimit::max(customization::BANNER_MAX_BYTES + customization::MULTIPART_OVERHEAD),
            ),
        )
        .route("/auth/password", post(auth::change_password))
        .route("/auth/sessions", get(auth::sessions).delete(auth::revoke_other_sessions))
        .route("/auth/sessions/{id}", delete(auth::revoke_session))
        // Konto-Sicherheit (Paket G): 2FA-Anmeldung, E-Mail, Passwort vergessen, 2FA, OIDC
        .route("/auth/login/mfa", post(auth::login_mfa))
        .route("/auth/login/mfa/email", post(auth::login_mfa_email))
        .route("/auth/security", get(security::overview))
        .route("/auth/email", put(security::set_email).delete(security::delete_email))
        .route("/auth/email/verify", post(security::verify_email))
        .route("/auth/password/forgot", post(security::forgot_password))
        .route("/auth/mfa/totp/setup", post(security::totp_setup))
        .route("/auth/mfa/totp/enable", post(security::totp_enable))
        .route("/auth/mfa/totp", delete(security::totp_disable))
        .route("/auth/mfa/email", post(security::email_mfa_enable).delete(security::email_mfa_disable))
        .route("/auth/mfa/email-code", post(security::send_reauth_code))
        .route("/auth/mfa/recovery-codes", post(security::regenerate_recovery_codes))
        .route("/auth/oidc/signup", post(oidc::signup_info))
        .route("/auth/oidc/register", post(oidc::register))
        .route("/auth/oidc/{provider}/start", post(oidc::start))
        .route("/auth/oidc/{provider}/callback", get(oidc::callback))
        .route("/auth/identities/{provider}", delete(oidc::unlink))
        // Passwort-Reset-Links einlösen (Paket H, Links erzeugt die Instanz-Verwaltung)
        .route("/auth/password-reset/verify", post(password_reset::verify))
        .route("/auth/password-reset/complete", post(password_reset::complete))
        // Speicher-Kontingente (Paket E)
        .route("/auth/me/usage", get(usage::me))
        .route("/communities/{id}/usage", get(usage::community))
        // Profile & Avatare
        .route("/users/{id}", get(users::profile))
        .route("/users/{id}/avatar/{avatar_id}", get(users::avatar))
        .route("/users/{id}/banner/{banner_id}", get(customization::user_banner))
        // Communities
        .route("/communities", get(communities::list).post(communities::create))
        .route("/communities/templates/discord/{code}", get(communities::discord_template_preview))
        .route(
            "/communities/{id}",
            get(communities::get).patch(communities::update).delete(communities::delete),
        )
        // Community-Gestaltung (Paket J)
        .route(
            "/communities/{id}/icon",
            post(customization::upload_icon).delete(customization::delete_icon).layer(DefaultBodyLimit::max(
                customization::ICON_MAX_BYTES + customization::MULTIPART_OVERHEAD,
            )),
        )
        .route(
            "/communities/{id}/banner",
            post(customization::upload_banner).delete(customization::delete_banner).layer(
                DefaultBodyLimit::max(customization::BANNER_MAX_BYTES + customization::MULTIPART_OVERHEAD),
            ),
        )
        .route("/communities/{id}/icon/{icon_id}", get(customization::icon))
        .route("/communities/{id}/banner/{banner_id}", get(customization::banner))
        .route("/communities/{id}/members", get(communities::members))
        .route(
            "/communities/{id}/members/{user_id}",
            delete(communities::remove_member).patch(communities::update_member),
        )
        .route("/communities/{id}/transfer", post(communities::transfer))
        // Banns
        .route("/communities/{id}/bans", get(bans::list))
        .route("/communities/{id}/bans/{user_id}", put(bans::ban).delete(bans::unban))
        .route(
            "/communities/{id}/members/{user_id}/roles/{role_id}",
            put(communities::add_member_role).delete(communities::remove_member_role),
        )
        .route("/communities/{id}/audit-log", get(communities::audit_log))
        .route("/communities/{id}/automod", get(automod::get).put(automod::update))
        .route("/communities/{id}/search", get(search::community))
        .route("/communities/{id}/channels", post(channels::create))
        // Kanal-Kategorien
        .route("/communities/{id}/categories", post(categories::create))
        .route("/communities/{id}/categories/order", axum::routing::patch(categories::reorder))
        .route("/categories/{id}", axum::routing::patch(categories::update).delete(categories::delete))
        .route("/communities/{id}/roles", post(roles::create))
        .route("/communities/{id}/roles/order", axum::routing::patch(roles::reorder))
        .route("/communities/{id}/invites", get(invites::list).post(invites::create))
        // Custom-Emotes (Paket I)
        .route(
            "/communities/{id}/emojis",
            get(emojis::list)
                .post(emojis::create)
                .layer(DefaultBodyLimit::max(crate::emojis::MAX_BYTES + 64 * 1024)),
        )
        .route(
            "/communities/{id}/emojis/{emoji_id}",
            axum::routing::patch(emojis::rename).delete(emojis::delete),
        )
        .route("/emojis", get(emojis::mine))
        .route("/emojis/{file}", get(emojis::image))
        // Kanäle & Nachrichten
        .route("/channels/{id}", axum::routing::patch(channels::update).delete(channels::delete))
        .route("/channels/{id}/overrides", get(channels::list_overrides))
        .route(
            "/channels/{id}/overrides/{role_id}",
            put(channels::put_override).delete(channels::delete_override),
        )
        .route("/channels/{id}/messages", get(messages::list).post(messages::create))
        // Ungelesen (Paket D)
        .route("/channels/{id}/read", put(read_states::mark_read))
        .route("/messages/{id}", axum::routing::patch(messages::update).delete(messages::delete))
        // Reaktionen und Anpinnen
        .route("/messages/{id}/reactions/{emoji}/@me", put(reactions::add).delete(reactions::remove))
        .route("/channels/{id}/pins", get(pins::list))
        .route("/channels/{id}/pins/{message_id}", put(pins::pin).delete(pins::unpin))
        // Dateianhänge (Limit prüft der Upload-Handler selbst gegen LYKOS_MAX_UPLOAD_MB)
        .route("/channels/{id}/attachments", post(attachments::upload).layer(DefaultBodyLimit::disable()))
        .route("/attachments/{id}", delete(attachments::delete))
        .route("/attachments/{id}/{filename}", get(attachments::download))
        // Direktnachrichten
        .route("/dms", get(dms::list).post(dms::create))
        .route("/dms/contacts", get(dms::contacts))
        .route("/dms/settings", get(dms::settings).patch(dms::update_settings))
        .route("/dms/search", get(search::dms))
        .route("/dms/{id}", axum::routing::patch(dms::update).delete(dms::leave))
        .route("/dms/{id}/pin", put(dms::pin).delete(dms::unpin))
        .route("/dms/{id}/recipients", post(dms::add))
        // Freundesystem
        .route("/relationships", get(relationships::list).post(relationships::create))
        .route("/relationships/settings", get(relationships::settings).patch(relationships::update_settings))
        .route("/relationships/{user_id}", put(relationships::put).delete(relationships::delete))
        // Rollen
        .route("/roles/{id}", axum::routing::patch(roles::update).delete(roles::delete))
        // Einladungen
        .route("/invites/{code}", get(invites::preview).delete(invites::delete))
        .route("/invites/{code}/accept", post(invites::accept))
        .route("/invites/{code}/icon/{icon_id}", get(customization::invite_icon))
        .route("/invites/{code}/banner/{banner_id}", get(customization::invite_banner))
        // Voice (LiveKit)
        .route("/channels/{id}/voice/join", post(voice::join))
        .route("/voice/state", post(voice::set_state))
        // Anrufe in DMs (Beitritt über …/voice/join)
        .route("/channels/{id}/call/decline", post(calls::decline))
        .route("/channels/{id}/call/ring", post(calls::ring_again))
        .route("/communities/{id}/voice-states", get(voice::community_states))
        .route("/livekit/webhook", post(voice::webhook))
        // Fehler- und Vorschlagsmeldungen (Paket K)
        .route("/feedback", post(feedback::create))
        .route("/feedback/mine", get(feedback::mine))
        .route("/feedback/suggestions", get(feedback::suggestions))
        .route("/feedback/{id}", get(feedback::get).delete(feedback::delete_own))
        .route(
            "/feedback/{id}/screenshot",
            get(feedback::screenshot)
                .post(feedback::upload_screenshot)
                .layer(DefaultBodyLimit::max(crate::feedback::SCREENSHOT_MAX_BYTES + 64 * 1024)),
        )
        .route("/feedback/{id}/vote", put(feedback::vote).delete(feedback::unvote))
        .route("/admin/feedback", get(feedback::admin_list))
        .route("/admin/feedback/settings", get(feedback::admin_settings))
        .route(
            "/admin/feedback/{id}",
            get(feedback::admin_get).patch(feedback::admin_update).delete(feedback::admin_delete),
        )
        .route("/admin/feedback/{id}/replies", post(feedback::admin_reply))
        .route("/admin/feedback/{id}/issue", post(feedback::admin_create_issue))
        // Instanz-Verwaltung (Paket H)
        .merge(admin::routes())
        // Eingehende Webhooks: Verwaltung (angemeldet, MANAGE_WEBHOOKS) …
        .route("/channels/{id}/webhooks", get(webhooks::list_channel).post(webhooks::create))
        .route("/communities/{id}/webhooks", get(webhooks::list_community))
        .route("/webhooks/{id}", get(webhooks::get).patch(webhooks::update).delete(webhooks::delete))
        .route("/webhooks/{id}/token", post(webhooks::regenerate_token))
        .route(
            "/webhooks/{id}/avatar",
            post(webhooks::upload_avatar)
                .delete(webhooks::delete_avatar)
                .layer(DefaultBodyLimit::max(users::AVATAR_MAX_BYTES + 64 * 1024)),
        )
        .route("/webhooks/{id}/avatar/{avatar_id}", get(webhooks::avatar))
        // … und Ausführen ohne Anmeldung (Token in der URL, Discord-kompatibel)
        .route("/webhooks/{id}/{token}", get(webhooks::info).post(webhooks::execute))
        .route(
            "/webhooks/{id}/{token}/messages/{message_id}",
            patch(webhooks::edit_message).delete(webhooks::delete_message),
        )
        // Bot-Konten
        .route("/bots", get(bots::list).post(bots::create))
        .route("/bots/{id}", get(bots::get).patch(bots::update).delete(bots::delete))
        .route("/bots/{id}/token", post(bots::regenerate_token))
        .route(
            "/bots/{id}/avatar",
            post(bots::upload_avatar)
                .delete(bots::delete_avatar)
                .layer(DefaultBodyLimit::max(users::AVATAR_MAX_BYTES + 64 * 1024)),
        )
        .route("/communities/{id}/bots", post(bots::add_to_community))
        // Zentrale Sperre: Bots dürfen Konto-, Sicherheits-, Freundes-, DM-Eröffnungs- und
        // Verwaltungs-Endpunkte nie nutzen (siehe `bots::bot_forbidden`).
        .layer(middleware::from_fn(bots::reject_forbidden))
}
