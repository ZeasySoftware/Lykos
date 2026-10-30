-- Nachrichten-Features: Antworten, Reaktionen, @Erwähnungen, Anpinnen.
-- Neue Rechte: ADD_REACTIONS (1 << 14, bekommen alle Standardrollen wie neue Communities),
-- MENTION_EVERYONE (1 << 15, bewusst nicht in der Standardrolle).
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

UPDATE roles SET permissions = permissions | 16384 WHERE is_default;

-- `reply_to_id` ist absichtlich kein Fremdschlüssel: Wird das Original gelöscht (oder von der
-- Aufbewahrung entfernt), bleibt die Antwort samt Verweis erhalten und zeigt „Ursprüngliche
-- Nachricht gelöscht“.
ALTER TABLE messages ADD COLUMN IF NOT EXISTS reply_to_id UUID;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS mention_everyone BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS pinned_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS messages_pinned ON messages (channel_id, pinned_at DESC) WHERE pinned_at IS NOT NULL;

-- Eine Reaktion pro Nutzer, Nachricht und Emoji.
CREATE TABLE IF NOT EXISTS message_reactions (
    message_id UUID        NOT NULL REFERENCES messages ON DELETE CASCADE,
    emoji      TEXT        NOT NULL,
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, emoji, user_id)
);

-- Erwähnte Nutzer (nur Mitglieder der Community zum Zeitpunkt des Sendens/Bearbeitens).
CREATE TABLE IF NOT EXISTS message_mentions (
    message_id UUID NOT NULL REFERENCES messages ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id)
);
CREATE INDEX IF NOT EXISTS message_mentions_user ON message_mentions (user_id, message_id);
