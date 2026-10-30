-- Moderation und Community-Einstellungen: Banns, AFK-Kanal, anpassbare Willkommensnachricht.
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

-- Banns: Gebannte sind keine Mitglieder mehr und können über Einladungen nicht wieder beitreten.
-- `banned_by` bleibt nach dem Löschen des Moderator-Kontos als NULL stehen; wird das gebannte Konto
-- gelöscht, verschwindet der Bann mit (die ID kann nie wieder vergeben werden).
CREATE TABLE IF NOT EXISTS community_bans (
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    user_id      UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    banned_by    UUID        REFERENCES users ON DELETE SET NULL,
    reason       TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, user_id)
);
CREATE INDEX IF NOT EXISTS community_bans_user ON community_bans (user_id);
CREATE INDEX IF NOT EXISTS community_bans_banned_by ON community_bans (banned_by);

-- Neues Recht BAN_MEMBERS (1 << 17): bekommt jede Rolle, die schon Mitglieder entfernen darf
-- (KICK_MEMBERS = 1 << 10). Rollen mit ADMINISTRATOR haben es ohnehin.
UPDATE roles SET permissions = permissions | (1 << 17)
 WHERE permissions & (1 << 10) <> 0 AND permissions & (1 << 17) = 0;

-- AFK-Kanal: Sprachkanal der Community, in den inaktive Nutzer wechseln; dort darf niemand sprechen,
-- streamen oder die Kamera nutzen. Die Zeit bis zum Wechsel misst der Client (Discord-Werte).
ALTER TABLE communities ADD COLUMN IF NOT EXISTS afk_channel_id UUID REFERENCES channels ON DELETE SET NULL;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS afk_timeout_seconds INT NOT NULL DEFAULT 300
    CHECK (afk_timeout_seconds IN (60, 300, 900, 1800, 3600));
CREATE INDEX IF NOT EXISTS communities_afk_channel ON communities (afk_channel_id);

-- Beitrittsmeldungen im Systemkanal abschaltbar; optional eigener Willkommenstext mit Platzhaltern
-- `{user}` (Erwähnung) und `{community}`. NULL = zufällige Standardsätze im Client.
ALTER TABLE communities ADD COLUMN IF NOT EXISTS join_messages_enabled BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS welcome_message TEXT
    CHECK (welcome_message IS NULL OR char_length(welcome_message) <= 500);
