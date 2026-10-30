-- Konto & Profil: Avatar, „Über mich“, Status-Text und das neue Recht CHANGE_NICKNAME (1 << 21).
-- Die Avatar-Datei liegt im Speicher-Backend unter dem Schlüssel `avatar_id` (wie Anhänge unter ihrer ID);
-- jeder Upload bekommt eine neue ID, damit Clients Avatare unbegrenzt cachen können.
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_id UUID;
ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_content_type TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS bio TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS status_text TEXT;

-- Eigenen Nickname ändern dürfen alle (wie neue Communities). MANAGE_NICKNAMES (1 << 22) bleibt Rollen vorbehalten.
UPDATE roles SET permissions = permissions | 2097152 WHERE is_default;
