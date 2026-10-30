-- Anrufe in Direktnachrichten (1:1 und Gruppen).
--
-- Ein DM-Kanal ist – wie ein Sprachkanal – ein LiveKit-Raum (Raumname = Kanal-ID). Wer im Anruf ist, steht
-- wie bisher in `voice_states`, dann ohne Community. Weiterhin gilt: ein Nutzer ist in höchstens einem
-- Sprachkanal oder Anruf (PK user_id).

ALTER TABLE voice_states ALTER COLUMN community_id DROP NOT NULL;

-- Klingeln: Wer gerade angerufen wird. Startet jemand einen Anruf (als Erster im Raum), klingelt es bei den
-- anderen Teilnehmern der DM bis `expires_at`, bis sie annehmen (beitreten) oder ablehnen oder der Anrufer
-- allein auflegt. Abgelaufene Zeilen räumt der Server ab; bis dahin zählen sie nicht.
CREATE TABLE call_rings (
    channel_id UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    caller_id  UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX call_rings_user ON call_rings (user_id);
CREATE INDEX call_rings_expires ON call_rings (expires_at);

-- Systemnachricht „Anruf verpasst“ in 1:1-DMs (Autor = Anrufer, Inhalt leer).
ALTER TABLE messages DROP CONSTRAINT IF EXISTS messages_kind_check;
ALTER TABLE messages ADD CONSTRAINT messages_kind_check CHECK (kind IN ('default', 'member_join', 'call_missed'));
