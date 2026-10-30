-- Direktnachrichten: Unterhaltungen anpinnen und Leute zu Gruppen hinzufügen.
--
-- Anpinnen gilt pro Nutzer (eigene Sicht): `pinned_at` in der eigenen Zeile von `dm_recipients`. Wer eine Gruppe
-- verlässt, verliert damit auch den Pin. Clients sortieren angepinnte Unterhaltungen nach diesem Zeitpunkt oben ein.
ALTER TABLE dm_recipients ADD COLUMN pinned_at TIMESTAMPTZ;

-- Systemnachricht „X hat Y hinzugefügt“ in Gruppen-DMs: Autor = wer hinzugefügt hat, die Hinzugefügten stehen in
-- `message_mentions` (Inhalt leer).
ALTER TABLE messages DROP CONSTRAINT IF EXISTS messages_kind_check;
ALTER TABLE messages ADD CONSTRAINT messages_kind_check
    CHECK (kind IN ('default', 'member_join', 'call_missed', 'recipient_add'));
