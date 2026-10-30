//! Konto-Löschung – gemeinsamer Kern für Selbstlöschung (`DELETE /auth/me`) und Admin-Löschung
//! (`DELETE /admin/users/{id}`). Vorbedingungen (Passwort, Besitz, letzter Admin) prüft der Aufrufer.

use uuid::Uuid;

use crate::{
    attachments,
    error::ApiResult,
    events::{self, Control, Scope},
    routes::{calls, dms, relationships},
    state::AppState,
    voice,
};

/// Löscht das Konto endgültig: wirft es aus Voice, löscht Sessions (Gateway trennt), Mitgliedschaften,
/// ungesendete Uploads, optional alle Nachrichten (`delete_messages`), und räumt leere DMs ab.
/// Der Nutzer darf keine Communities mehr besitzen (sonst schlägt der Fremdschlüssel fehl).
/// Eigene Bots werden vorher genauso gelöscht (Bots können keine Communities besitzen).
pub async fn delete_account(state: &AppState, user_id: Uuid, delete_messages: bool) -> ApiResult<()> {
    let bots: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE bot_owner_id = $1")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;
    for bot_id in bots {
        Box::pin(delete_account(state, bot_id, delete_messages)).await?;
    }

    let communities: Vec<Uuid> = sqlx::query_scalar("SELECT community_id FROM members WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;
    // Aus dem Voice werfen, solange Mitgliedschaft und Voice-Status noch existieren.
    for community_id in &communities {
        voice::member_removed(state, *community_id, user_id).await?;
    }
    // DM-Anruf: auflegen, Klingeln bei ihm und von ihm beenden.
    voice::disconnect_user(state, user_id).await?;
    calls::user_removed(state, user_id).await?;

    let mut tx = state.db.begin().await?;
    // Sessions bzw. bei Bots die Token-ID (dient im Gateway als Session).
    let sessions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE user_id = $1 UNION ALL SELECT token_id FROM bots WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    // Profilbilder (Avatar, Banner) werden mit dem Konto gelöscht.
    let (avatar, banner): (Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT avatar_id, banner_id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    // Nicht gesendete Uploads gehören niemandem mehr – sofort weg.
    let mut files: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM attachments WHERE uploader_id = $1 AND message_id IS NULL RETURNING id",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut deleted_messages: Vec<(Uuid, Uuid, Option<Uuid>)> = Vec::new();
    if delete_messages {
        files.extend(
            sqlx::query_scalar::<_, Uuid>(
                "SELECT a.id FROM attachments a JOIN messages m ON m.id = a.message_id WHERE m.author_id = $1",
            )
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?,
        );
        deleted_messages = sqlx::query_as(
            "DELETE FROM messages m USING channels c
             WHERE m.channel_id = c.id AND m.author_id = $1
             RETURNING m.id, m.channel_id, c.community_id",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    }
    let dms: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM dm_recipients WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    // Freunde, offene Anfragen und Blockierende erfahren danach, dass die Beziehung weg ist.
    let counterparts = relationships::counterparts(&mut *tx, user_id).await?;
    // Kaskaden: Sessions, Mitgliedschaften, Rollen, Lesestände, Voice-Status, DM-Teilnahmen, Beziehungen
    // (Freunde, Anfragen, Blockaden in beide Richtungen), Konto-Sicherheit
    // (Paket G: E-Mail samt ausstehender Bestätigung, 2FA, Wiederherstellungs- und E-Mail-Codes,
    // Anmelde-Challenges, OIDC-Identitäten und laufende Verknüpfungen, Reset-Links). Verbleibende
    // Nachrichten, Anhänge, Einladungen, Audit-Einträge und Meldungen (Feedback, Paket K) behalten ihre
    // Zeilen, nur der Verweis wird NULL.
    sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&mut *tx).await?;
    // DMs ohne Teilnehmer räumen wir samt Verlauf ab (Dateien holt der Hintergrund-Job).
    let emptied: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM channels c WHERE c.id = ANY($1)
           AND NOT EXISTS (SELECT 1 FROM dm_recipients r WHERE r.channel_id = c.id)
         RETURNING c.id",
    )
    .bind(&dms)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for channel_id in dms.iter().filter(|id| !emptied.contains(id)) {
        dms::announce_update(state, *channel_id).await?;
    }
    relationships::announce_deleted(state, user_id, &counterparts);

    files.extend(avatar);
    files.extend(banner);
    attachments::delete_files(state.storage.as_ref(), &files).await;

    for (id, channel_id, community_id) in deleted_messages {
        state.events.publish(
            events::channel(community_id, channel_id),
            "MESSAGE_DELETE",
            serde_json::json!({ "id": id, "channel_id": channel_id, "community_id": community_id }),
        );
    }
    for community_id in communities {
        state.events.publish(
            events::community(community_id),
            "MEMBER_REMOVE",
            serde_json::json!({ "community_id": community_id, "user_id": user_id }),
        );
        state.events.control(Scope::User(user_id), Control::Left { user_id, community_id });
    }
    for session_id in sessions {
        state.events.control(Scope::Session(session_id), Control::SessionRevoked);
    }
    Ok(())
}
