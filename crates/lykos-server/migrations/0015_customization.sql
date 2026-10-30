-- Paket J: Community- und Profil-Gestaltung.
-- Bilder (Community-Icon und -Banner, Profilbanner) liegen wie Avatare im Speicher-Backend unter ihrer ID;
-- jeder Upload bekommt eine neue ID, damit Clients die Bilder unbegrenzt cachen können.
-- Farben sind 24-Bit-RGB als INT (0xRRGGBB) wie `roles.color`.
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

ALTER TABLE communities ADD COLUMN IF NOT EXISTS icon_id UUID;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS icon_content_type TEXT;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS banner_id UUID;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS banner_content_type TEXT;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS accent_color INT
    CHECK (accent_color IS NULL OR accent_color BETWEEN 0 AND 16777215);
-- Kanal für Systemnachrichten („X ist beigetreten“); NULL = keine.
ALTER TABLE communities ADD COLUMN IF NOT EXISTS system_channel_id UUID REFERENCES channels ON DELETE SET NULL;

ALTER TABLE users ADD COLUMN IF NOT EXISTS banner_id UUID;
ALTER TABLE users ADD COLUMN IF NOT EXISTS banner_content_type TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS banner_color INT
    CHECK (banner_color IS NULL OR banner_color BETWEEN 0 AND 16777215);
ALTER TABLE users ADD COLUMN IF NOT EXISTS accent_color INT
    CHECK (accent_color IS NULL OR accent_color BETWEEN 0 AND 16777215);
ALTER TABLE users ADD COLUMN IF NOT EXISTS pronouns TEXT;

-- Art der Nachricht: `default` (vom Nutzer geschrieben) oder Systemnachricht (`member_join`).
-- Systemnachrichten haben leeren Inhalt; `author_id` ist der betroffene Nutzer.
ALTER TABLE messages ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'default';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'messages_kind_check') THEN
        ALTER TABLE messages ADD CONSTRAINT messages_kind_check CHECK (kind IN ('default', 'member_join'));
    END IF;
END $$;
