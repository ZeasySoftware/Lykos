-- Paket C: Dateianhänge. Die Bytes liegen im Speicher-Backend unter dem Schlüssel `id`
-- (nie unter dem Nutzer-Dateinamen). Uploads sind zunächst unverknüpft (message_id NULL) und werden
-- beim Senden einer Nachricht verknüpft; unverknüpfte Uploads räumt der Aufbewahrungs-Job nach 24 h weg.
-- Kaskaden entfernen nur die Zeilen – verwaiste Dateien gleicht der Job mit dem Speicher ab.

CREATE TABLE attachments (
    id           UUID PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    channel_id   UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    uploader_id  UUID REFERENCES users ON DELETE SET NULL,
    message_id   UUID REFERENCES messages ON DELETE CASCADE,
    filename     TEXT        NOT NULL,
    content_type TEXT        NOT NULL,
    size_bytes   BIGINT      NOT NULL CHECK (size_bytes >= 0),
    width        INT,
    height       INT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX attachments_message ON attachments (message_id, id) WHERE message_id IS NOT NULL;
CREATE INDEX attachments_unlinked ON attachments (created_at) WHERE message_id IS NULL;
CREATE INDEX attachments_channel ON attachments (channel_id);
