-- Kanal-Kategorien wie bei Discord: eine Community gruppiert ihre Kanäle in einklappbare Abschnitte.
-- Kategorien tragen keine eigenen Rechte – Sichtbarkeit ergibt sich aus den Kanälen (eine Kategorie sieht,
-- wer mindestens einen ihrer Kanäle sehen darf oder `MANAGE_CHANNELS` hat). Kanäle ohne Kategorie
-- (`category_id IS NULL`) erscheinen wie bisher oben unter „Textkanäle“/„Sprachkanäle“.

CREATE TABLE channel_categories (
    id           UUID PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    -- Reihenfolge in der Seitenleiste, aufsteigend.
    position     INT         NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX channel_categories_community ON channel_categories (community_id);

-- Gelöschte Kategorie: ihre Kanäle wandern zurück nach oben (ohne Kategorie). Dass Kategorie und Kanal zur
-- selben Community gehören, prüft die Anwendung.
ALTER TABLE channels ADD COLUMN category_id UUID REFERENCES channel_categories ON DELETE SET NULL;
CREATE INDEX channels_category ON channels (category_id) WHERE category_id IS NOT NULL;
