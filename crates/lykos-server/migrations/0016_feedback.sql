-- Paket K: Fehler- und Vorschlagsmeldungen aus der App.
-- Fehlerberichte sehen nur Melder und Instanz-Admins; Vorschläge sind für alle angemeldeten Nutzer der
-- Instanz sichtbar (ohne Melder, ohne technische Infos und Screenshot) und können unterstützt werden („+1“).
-- Der optionale Screenshot liegt im Speicher-Backend unter `screenshot_id` (wie Anhänge unter ihrer ID).
-- Wird ein Konto gelöscht, bleiben seine Meldungen ohne Melder erhalten (`reporter_id = NULL`).

CREATE TABLE feedback (
    id                      UUID PRIMARY KEY,
    -- Fortlaufende Meldungsnummer für Rückfragen („Meldung #12“) und Gitea-Issues (ohne Nutzernamen).
    number                  BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
    reporter_id             UUID REFERENCES users (id) ON DELETE SET NULL,
    kind                    TEXT        NOT NULL CHECK (kind IN ('bug', 'suggestion', 'other')),
    status                  TEXT        NOT NULL DEFAULT 'neu'
                                CHECK (status IN ('neu', 'in_arbeit', 'erledigt', 'abgelehnt', 'duplikat')),
    title                   TEXT        NOT NULL,
    description             TEXT        NOT NULL,
    -- Technische Infos (nur mit Opt-in des Melders), serverseitig auf bekannte Felder reduziert.
    tech_info               JSONB,
    screenshot_id           UUID,
    screenshot_content_type TEXT,
    -- Link auf das per Klick angelegte Gitea-Issue.
    issue_url               TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX feedback_reporter ON feedback (reporter_id, created_at) WHERE reporter_id IS NOT NULL;
CREATE INDEX feedback_kind_status ON feedback (kind, status);

-- Unterstützung von Vorschlägen: höchstens eine Stimme pro Nutzer.
CREATE TABLE feedback_votes (
    feedback_id UUID        NOT NULL REFERENCES feedback (id) ON DELETE CASCADE,
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (feedback_id, user_id)
);

-- Antworten der Admins. `internal = true` sind Notizen, die nur Admins sehen.
CREATE TABLE feedback_replies (
    id          UUID PRIMARY KEY,
    feedback_id UUID        NOT NULL REFERENCES feedback (id) ON DELETE CASCADE,
    author_id   UUID REFERENCES users (id) ON DELETE SET NULL,
    body        TEXT        NOT NULL,
    internal    BOOLEAN     NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX feedback_replies_feedback ON feedback_replies (feedback_id, id);
