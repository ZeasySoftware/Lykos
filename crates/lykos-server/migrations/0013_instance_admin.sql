-- Paket H: Instanz-Verwaltung für Instanz-Admins.
--
-- * Gesperrte Konten (`users.disabled_at`): Anmeldung verweigert, Sessions werden beim Sperren gelöscht.
-- * Instanz-Einladungen: Registrierungs-Einladungen ohne Community-Bindung (optional mit automatischem
--   Beitritt zu einer Community). Codes sind 12 Zeichen lang, Community-Einladungen 8 – die Namensräume
--   überschneiden sich also nicht, beide laufen über `/invite/{code}`.
-- * Instanzweites Audit-Log, getrennt von `audit_log` (dort ist `community_id` Pflicht und die Einträge
--   verschwinden mit der Community – Admin-Aktionen sollen das überleben).
-- * Einmal-Links zum Zurücksetzen des Passworts. `kind` unterscheidet die Herkunft: `admin_reset` = vom
--   Admin erzeugt und selbst weitergegeben (ohne E-Mail-Versand). Weitere Arten (z. B. E-Mail) können die
--   Tabelle mitbenutzen, indem sie den CHECK erweitern.
-- * Laufzeit-Einstellungen liegen weiter in `instance_settings` (Schlüssel siehe `instance_settings.rs`),
--   Umgebungsvariablen bleiben die Standardwerte.

ALTER TABLE users ADD COLUMN IF NOT EXISTS disabled_at TIMESTAMPTZ;

CREATE TABLE instance_invites (
    code         TEXT PRIMARY KEY,
    created_by   UUID REFERENCES users ON DELETE SET NULL,
    note         TEXT,
    -- Optional: neue Nutzer treten dieser Community automatisch bei.
    community_id UUID REFERENCES communities ON DELETE SET NULL,
    max_uses     INT CHECK (max_uses > 0),
    uses         INT         NOT NULL DEFAULT 0,
    expires_at   TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Wer sich mit welcher Instanz-Einladung registriert hat.
CREATE TABLE instance_invite_uses (
    code    TEXT        NOT NULL REFERENCES instance_invites ON DELETE CASCADE,
    user_id UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (code, user_id)
);
CREATE INDEX instance_invite_uses_user ON instance_invite_uses (user_id);

CREATE TABLE instance_audit_log (
    id         UUID PRIMARY KEY,
    actor_id   UUID REFERENCES users ON DELETE SET NULL,
    action     TEXT        NOT NULL,
    target_id  UUID,
    data       JSONB       NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX instance_audit_log_id ON instance_audit_log (id DESC);

-- Gespeichert wird nur der SHA-256-Hash des Tokens (wie bei Sessions).
CREATE TABLE password_reset_tokens (
    id         UUID PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    token_hash BYTEA       NOT NULL UNIQUE,
    kind       TEXT        NOT NULL CHECK (kind IN ('admin_reset')),
    created_by UUID REFERENCES users ON DELETE SET NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX password_reset_tokens_user ON password_reset_tokens (user_id);
