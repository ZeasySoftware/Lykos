-- AutoMod pro Community: Regeln, die der Server beim Erstellen und Bearbeiten von Nachrichten in
-- Community-Kanälen prüft (nicht in DMs). Je Community höchstens eine Regel pro Art; fehlende Zeilen
-- bedeuten „aus“ mit Voreinstellungen (siehe `automod.rs`). Der Server cacht die Regeln im Speicher.
--
-- `config` hängt von der Art ab:
--   keyword  {"terms": [...], "allow": [...]}
--   links    {"allowed_domains": [...], "block_foreign_invites": true}
--   mentions {"max_mentions": 5}
--   spam     {"max_messages": 5, "per_seconds": 5, "max_duplicates": 3}
CREATE TABLE automod_rules (
    community_id       UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    kind               TEXT        NOT NULL CHECK (kind IN ('keyword', 'links', 'mentions', 'spam')),
    enabled            BOOLEAN     NOT NULL DEFAULT false,
    config             JSONB       NOT NULL DEFAULT '{}',
    -- Aktionen: Nachricht blockieren, Alarm in den Protokollkanal, Eintrag ins Audit-Log.
    action_block       BOOLEAN     NOT NULL DEFAULT true,
    action_alert       BOOLEAN     NOT NULL DEFAULT false,
    action_audit       BOOLEAN     NOT NULL DEFAULT false,
    alert_channel_id   UUID        REFERENCES channels ON DELETE SET NULL,
    -- Eigene Meldung an den Absender beim Blockieren; NULL = Standardtext.
    custom_message     TEXT,
    -- Ausnahmen: Mitglieder mit einer dieser Rollen bzw. Nachrichten in diesen Kanälen.
    exempt_role_ids    UUID[]      NOT NULL DEFAULT '{}',
    exempt_channel_ids UUID[]      NOT NULL DEFAULT '{}',
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, kind)
);
CREATE INDEX automod_rules_alert_channel ON automod_rules (alert_channel_id);

-- Systemnachricht `automod_alert` im Protokollkanal: Autor = betroffenes Mitglied, Inhalt = JSON mit Regel,
-- Kanal, Aktion und gekürztem Text. Der Constraint wird um die neue Art erweitert, ohne die bestehende
-- Liste zu kennen (andere Migrationen können weitere Arten ergänzt haben).
DO $$
DECLARE
    def TEXT;
BEGIN
    SELECT pg_get_constraintdef(oid) INTO def FROM pg_constraint
    WHERE conname = 'messages_kind_check' AND conrelid = 'messages'::regclass;
    IF def IS NULL THEN
        ALTER TABLE messages ADD CONSTRAINT messages_kind_check
            CHECK (kind IN ('default', 'member_join', 'call_missed', 'automod_alert'));
    ELSIF position('automod_alert' IN def) = 0 THEN
        ALTER TABLE messages DROP CONSTRAINT messages_kind_check;
        -- `def` hat die Form `CHECK ((kind = ANY (ARRAY['default'::text, …])))`.
        EXECUTE 'ALTER TABLE messages ADD CONSTRAINT messages_kind_check '
            || replace(def, 'ARRAY[', 'ARRAY[''automod_alert''::text, ');
    END IF;
END $$;
