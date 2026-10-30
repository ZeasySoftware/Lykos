-- Performance-Audit (1 Mio. Nachrichten, 5.000 Nutzer): fehlende Indizes.
--
-- sqlx führt Migrationen in einer Transaktion aus, `CREATE INDEX CONCURRENTLY` geht dort nicht.
-- Auf großen Instanzen sperrt der Aufbau die Tabelle `messages` daher kurz für Schreibzugriffe
-- (bei 1 Mio. Nachrichten im Bereich von Sekunden). Wer das vermeiden will, legt die Indizes vorher
-- von Hand mit `CONCURRENTLY` an – dank `IF NOT EXISTS` überspringt die Migration sie dann.

-- Ungelesene `@everyone`-Erwähnungen je Kanal (`read_states::mention_counts`): nur die wenigen
-- Nachrichten mit `mention_everyone`, sortiert nach ID für `id > last_read_message_id`.
CREATE INDEX IF NOT EXISTS messages_mention_everyone ON messages (channel_id, id) WHERE mention_everyone;

-- Fremdschlüssel ohne Index: Löschen eines Nutzers/einer Rolle (ON DELETE SET NULL/CASCADE) müsste
-- sonst die ganze Tabelle durchsuchen.
CREATE INDEX IF NOT EXISTS messages_author ON messages (author_id) WHERE author_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS message_reactions_user ON message_reactions (user_id);
CREATE INDEX IF NOT EXISTS audit_log_actor ON audit_log (actor_id) WHERE actor_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS channel_overrides_role ON channel_overrides (role_id);
CREATE INDEX IF NOT EXISTS custom_emojis_uploader ON custom_emojis (uploader_id) WHERE uploader_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS feedback_votes_user ON feedback_votes (user_id);
