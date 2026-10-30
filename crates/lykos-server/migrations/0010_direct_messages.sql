-- MVP 2, Paket C: Direktnachrichten (1:1 und Gruppen bis 10 Personen) und Nachrichtensuche.
--
-- DMs sind gewöhnliche Kanäle ohne Community (`community_id IS NULL`, `kind` = 'dm' | 'group_dm').
-- Dadurch funktionieren Nachrichten, Anhänge, Lesestand und Tipp-Anzeige ohne eigene Code-Pfade;
-- später können Voice-Calls denselben Kanal als LiveKit-Raum nutzen. Wer teilnimmt, steht in
-- `dm_recipients` – das ist auch die Mitgliederliste einer späteren MLS-Gruppe (E2EE, RFC 9420).

ALTER TABLE channels ALTER COLUMN community_id DROP NOT NULL;
ALTER TABLE channels DROP CONSTRAINT channels_kind_check;
ALTER TABLE channels ADD CONSTRAINT channels_kind_check CHECK (kind IN ('text', 'voice', 'dm', 'group_dm'));
-- Genau die DM-Arten haben keine Community.
ALTER TABLE channels ADD CONSTRAINT channels_dm_without_community
    CHECK ((community_id IS NULL) = (kind IN ('dm', 'group_dm')));
-- Nur 1:1-DMs: "<kleinere user_id>:<größere user_id>" – verhindert doppelte DMs zwischen zwei Personen.
ALTER TABLE channels ADD COLUMN dm_key TEXT UNIQUE;

CREATE TABLE dm_recipients (
    channel_id UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    joined_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX dm_recipients_user ON dm_recipients (user_id);

-- Anhänge in DMs gehören zu keiner Community (Aufbewahrungsfristen gelten dort nicht).
ALTER TABLE attachments ALTER COLUMN community_id DROP NOT NULL;

-- Nutzer-Einstellung: Dürfen Mitglieder gemeinsamer Communities eine DM eröffnen? (Standard: ja)
ALTER TABLE users ADD COLUMN allow_dms BOOLEAN NOT NULL DEFAULT true;

-- Volltextsuche: deutsche Wortstämme plus `simple` als Rückfall (Stoppwörter, Namen, Fremdwörter).
-- Die Ausdrücke müssen exakt denen in `routes/search.rs` entsprechen, damit die Indizes greifen.
CREATE INDEX messages_search_german ON messages USING GIN (to_tsvector('german', content));
CREATE INDEX messages_search_simple ON messages USING GIN (to_tsvector('simple', content));
