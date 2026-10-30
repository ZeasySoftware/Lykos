-- Paket E: Speicher-Kontingente für Nachrichten-Anhänge.
-- Pro Nutzer gilt die Instanz-Einstellung LYKOS_USER_QUOTA_MB (nicht in der Datenbank).
-- Pro Community optional ein eigenes Kontingent; NULL = unbegrenzt (nur das Nutzer-Kontingent zählt).
-- Belegt ist die Summe aller Anhänge (auch noch nicht gesendeter Uploads) des Nutzers bzw. der Community.

ALTER TABLE communities
    ADD COLUMN upload_quota_mb INT CHECK (upload_quota_mb IS NULL OR upload_quota_mb > 0);

CREATE INDEX attachments_uploader ON attachments (uploader_id) WHERE uploader_id IS NOT NULL;
CREATE INDEX attachments_community ON attachments (community_id);
