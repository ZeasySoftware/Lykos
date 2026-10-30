-- Freundesystem: Freundschaften, Anfragen und Blockieren.
--
-- Eine Zeile beschreibt die Beziehung aus Sicht von `user_id` zu `target_id`. Freundschaften und Anfragen
-- stehen immer paarweise in beiden Richtungen (`friend`/`friend` bzw. `pending_out`/`pending_in`);
-- Blockieren ist einseitig (`blocked`) und entfernt die Gegenzeile – außer der andere blockiert ebenfalls.
-- Die Zeilen verschwinden mit dem Konto (ON DELETE CASCADE).

CREATE TABLE relationships (
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    target_id  UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    kind       TEXT        NOT NULL CHECK (kind IN ('friend', 'pending_out', 'pending_in', 'blocked')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, target_id),
    CHECK (user_id <> target_id)
);
-- Gegenrichtung („wer hat eine Beziehung zu mir?“) für Blockprüfungen, Profilsicht und Konto-Löschen.
CREATE INDEX relationships_target ON relationships (target_id);

-- Wer darf mir Freundschaftsanfragen schicken? (Standard: alle)
ALTER TABLE users ADD COLUMN friend_requests TEXT NOT NULL DEFAULT 'everyone'
    CHECK (friend_requests IN ('everyone', 'mutual_communities', 'nobody'));
