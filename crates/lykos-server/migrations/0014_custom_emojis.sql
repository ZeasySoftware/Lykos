-- Paket I: Custom-Emotes pro Community und Reaktionen damit.
-- Neues Recht MANAGE_EMOJIS (1 << 16) – bewusst nicht in der Standardrolle, daher keine Rollen-Anpassung.
-- Die Bilddatei liegt im Speicher-Backend unter dem Schlüssel `id` (wie Anhänge und Avatare).
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

CREATE TABLE IF NOT EXISTS custom_emojis (
    id           UUID        PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    animated     BOOLEAN     NOT NULL DEFAULT false,
    content_type TEXT        NOT NULL,
    size_bytes   INT         NOT NULL,
    uploader_id  UUID        REFERENCES users ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Namen sind pro Community eindeutig (ohne Groß/Klein-Unterschied).
CREATE UNIQUE INDEX IF NOT EXISTS custom_emojis_name ON custom_emojis (community_id, lower(name));

-- Reaktionen mit Custom-Emotes: `emoji` enthält dann die Emote-ID (als Text), `emoji_name` den Namen zum
-- Zeitpunkt der Reaktion – Platzhalter `:name:`, falls das Emote später gelöscht wird.
ALTER TABLE message_reactions ADD COLUMN IF NOT EXISTS emoji_name TEXT;
