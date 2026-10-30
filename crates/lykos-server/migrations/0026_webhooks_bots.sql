-- Eingehende Webhooks und Bot-Konten.
--
-- * Neues Recht MANAGE_WEBHOOKS (1 << 18) – bewusst nicht in der Standardrolle, daher keine Datenänderung.
-- * Webhooks gehören zu einem Textkanal einer Community. Gespeichert wird nur der SHA-256-Hash des Tokens
--   (wie bei Sessions); der Klartext existiert nur in der Antwort beim Anlegen bzw. Neu-Erzeugen.
-- * Webhook-Nachrichten haben keinen Autor (`author_id` NULL), sondern `webhook_id` + Anzeigename. Wird der
--   Webhook gelöscht, bleiben die Nachrichten samt Namen erhalten (`webhook_id` → NULL).
-- * Embeds (Discord-kompatible Karten) liegen als bereinigtes JSON in `messages.embeds`; Bild-URLs werden
--   gar nicht erst gespeichert.
-- * Bots sind Nutzer mit `is_bot = true` und Besitzer (`bot_owner_id`), ohne Passwort, E-Mail oder 2FA.
--   Ihr Token (gehasht) steht in `bots`. Löscht der Besitzer sein Konto, gehen seine Bots mit (die
--   Anwendung räumt sie vorher ordentlich ab, der Fremdschlüssel ist nur doppelter Boden).
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

ALTER TABLE users ADD COLUMN IF NOT EXISTS is_bot BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS bot_owner_id UUID REFERENCES users ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS users_bot_owner ON users (bot_owner_id) WHERE bot_owner_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS bots (
    user_id     UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    description TEXT,
    -- Öffentliche Bots dürfen auch Fremde (mit MANAGE_COMMUNITY) zu ihrer Community hinzufügen.
    is_public   BOOLEAN     NOT NULL DEFAULT false,
    token_hash  BYTEA       NOT NULL UNIQUE,
    -- Wechselt mit jedem neuen Token und dient dem Gateway als Session-ID (Widerruf trennt Verbindungen).
    token_id    UUID        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS webhooks (
    id                  UUID PRIMARY KEY,
    community_id        UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    channel_id          UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    name                TEXT        NOT NULL,
    avatar_id           UUID,
    avatar_content_type TEXT,
    token_hash          BYTEA       NOT NULL UNIQUE,
    created_by          UUID REFERENCES users ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS webhooks_channel ON webhooks (channel_id);
CREATE INDEX IF NOT EXISTS webhooks_community ON webhooks (community_id);

ALTER TABLE messages ADD COLUMN IF NOT EXISTS webhook_id UUID REFERENCES webhooks ON DELETE SET NULL;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS webhook_name TEXT;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS embeds JSONB NOT NULL DEFAULT '[]';
CREATE INDEX IF NOT EXISTS messages_webhook ON messages (webhook_id) WHERE webhook_id IS NOT NULL;
