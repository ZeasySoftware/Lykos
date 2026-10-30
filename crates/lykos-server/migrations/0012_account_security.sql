-- Paket G: Konto-Sicherheit. Alles optional – ohne E-Mail, 2FA oder OIDC bleibt ein Konto wie bisher.
-- Gespeichert werden nur Hashes von Tokens und Codes; Klartext existiert nur in der Mail bzw. Antwort.

-- Konten, die nur per OIDC angelegt wurden, haben (noch) kein Passwort.
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- Bestätigte E-Mail-Adresse (nur zur Kontowiederherstellung und für 2FA per E-Mail).
-- Unbestätigte Adressen stehen ausschließlich in email_verifications.
ALTER TABLE users ADD COLUMN email TEXT;
CREATE UNIQUE INDEX users_email_lower ON users (lower(email)) WHERE email IS NOT NULL;

-- Ausstehende Bestätigung einer (neuen) Adresse; höchstens eine pro Konto.
CREATE TABLE email_verifications (
    user_id    UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    email      TEXT        NOT NULL,
    token_hash BYTEA       NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Einmalige Reset-Links (30 min).
CREATE TABLE password_resets (
    token_hash BYTEA PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX password_resets_user ON password_resets (user_id);

-- Zwei-Faktor-Einstellungen. 2FA ist aktiv, wenn totp_secret gesetzt oder email_enabled ist.
CREATE TABLE user_mfa (
    user_id             UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    -- Geheimnis der Authenticator-App (verlässt den Server nach der Einrichtung nie wieder).
    totp_secret         BYTEA,
    -- Zuletzt akzeptierter Zeitschritt (Replay-Schutz).
    totp_last_step      BIGINT,
    -- Geheimnis während der Einrichtung, bis der erste Code bestätigt ist.
    totp_pending_secret BYTEA,
    email_enabled       BOOLEAN     NOT NULL DEFAULT false,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Einmal-Wiederherstellungscodes (SHA-256).
CREATE TABLE mfa_recovery_codes (
    user_id   UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    code_hash BYTEA       NOT NULL,
    used_at   TIMESTAMPTZ,
    PRIMARY KEY (user_id, code_hash)
);

-- Aktueller E-Mail-Code (Anmeldung oder Bestätigung sensibler Aktionen); höchstens einer pro Konto.
CREATE TABLE mfa_email_codes (
    user_id    UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    code_hash  BYTEA       NOT NULL,
    attempts   INT         NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ NOT NULL
);

-- Zweiter Anmeldeschritt: Passwort (oder OIDC) war richtig, der 2FA-Code fehlt noch (5 min).
CREATE TABLE mfa_challenges (
    token_hash    BYTEA PRIMARY KEY,
    user_id       UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    session_label TEXT,
    attempts      INT         NOT NULL DEFAULT 0,
    expires_at    TIMESTAMPTZ NOT NULL
);

-- Verknüpfte OIDC-Identitäten. Gespeichert werden nur Anbieter und Subject, keine Profildaten.
CREATE TABLE user_identities (
    provider     TEXT        NOT NULL,
    subject      TEXT        NOT NULL,
    user_id      UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    PRIMARY KEY (provider, subject),
    UNIQUE (user_id, provider)
);

-- Laufende OIDC-Anmeldungen (state → nonce, PKCE-Verifier; 10 min).
CREATE TABLE oidc_flows (
    state_hash    BYTEA PRIMARY KEY,
    provider      TEXT        NOT NULL,
    nonce         TEXT        NOT NULL,
    pkce_verifier TEXT        NOT NULL,
    -- Gesetzt beim Verknüpfen mit einem bestehenden Konto.
    link_user_id  UUID REFERENCES users ON DELETE CASCADE,
    session_label TEXT,
    expires_at    TIMESTAMPTZ NOT NULL
);

-- Identität ohne Konto: wartet darauf, dass ein Benutzername gewählt wird (15 min).
CREATE TABLE oidc_signups (
    token_hash BYTEA PRIMARY KEY,
    provider   TEXT        NOT NULL,
    subject    TEXT        NOT NULL,
    -- Vom Anbieter als bestätigt gemeldete Adresse; wird nur nach Zustimmung übernommen.
    email      TEXT,
    session_label TEXT,
    expires_at TIMESTAMPTZ NOT NULL
);
