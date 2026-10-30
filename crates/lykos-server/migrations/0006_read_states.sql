-- MVP 1, Paket D: Lesestand pro Nutzer und Kanal (Ungelesen-Markierung).
-- Präsenz und Tipp-Anzeige sind bewusst flüchtig und haben keine Tabelle.
--
-- `last_read_message_id` ist absichtlich kein Fremdschlüssel auf `messages`: Wird die zuletzt
-- gelesene Nachricht gelöscht (oder von der Aufbewahrung entfernt), bleibt der Lesestand erhalten.
-- Da IDs UUIDv7 sind, gilt alles mit größerer ID als ungelesen.

CREATE TABLE read_states (
    user_id              UUID NOT NULL REFERENCES users ON DELETE CASCADE,
    channel_id           UUID NOT NULL REFERENCES channels ON DELETE CASCADE,
    last_read_message_id UUID NOT NULL,
    PRIMARY KEY (user_id, channel_id)
);
CREATE INDEX read_states_channel ON read_states (channel_id);
