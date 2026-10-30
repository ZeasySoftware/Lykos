-- Phase 1: Accounts, Sessions, Communities, Rollen, Kanäle, Nachrichten, Einladungen, Audit-Log.
-- IDs sind UUIDv7 und werden in der Anwendung erzeugt (zeitlich sortierbar).

CREATE TABLE users (
    id                UUID PRIMARY KEY,
    username          TEXT        NOT NULL,
    display_name      TEXT,
    password_hash     TEXT        NOT NULL,
    is_instance_admin BOOLEAN     NOT NULL DEFAULT false,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_username_lower ON users (lower(username));

-- Gespeichert wird nur der SHA-256-Hash des Tokens. Bewusst keine IP-Adressen.
CREATE TABLE sessions (
    id           UUID PRIMARY KEY,
    user_id      UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    token_hash   BYTEA       NOT NULL UNIQUE,
    label        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE communities (
    id                     UUID PRIMARY KEY,
    name                   TEXT        NOT NULL,
    owner_id               UUID        NOT NULL REFERENCES users,
    -- NULL = Nachrichten unbegrenzt aufbewahren
    message_retention_days INT CHECK (message_retention_days > 0),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE roles (
    id           UUID PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    permissions  BIGINT      NOT NULL DEFAULT 0,
    position     INT         NOT NULL DEFAULT 0,
    color        INT,
    is_default   BOOLEAN     NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX roles_community ON roles (community_id);
CREATE UNIQUE INDEX roles_one_default ON roles (community_id) WHERE is_default;

CREATE TABLE members (
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    user_id      UUID        NOT NULL REFERENCES users ON DELETE CASCADE,
    nickname     TEXT,
    joined_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, user_id)
);
CREATE INDEX members_user ON members (user_id);

CREATE TABLE member_roles (
    community_id UUID NOT NULL,
    user_id      UUID NOT NULL,
    role_id      UUID NOT NULL REFERENCES roles ON DELETE CASCADE,
    PRIMARY KEY (community_id, user_id, role_id),
    FOREIGN KEY (community_id, user_id) REFERENCES members ON DELETE CASCADE
);
CREATE INDEX member_roles_role ON member_roles (role_id);

CREATE TABLE channels (
    id           UUID PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    kind         TEXT        NOT NULL CHECK (kind IN ('text', 'voice')),
    topic        TEXT,
    position     INT         NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX channels_community ON channels (community_id);

CREATE TABLE channel_overrides (
    channel_id UUID   NOT NULL REFERENCES channels ON DELETE CASCADE,
    role_id    UUID   NOT NULL REFERENCES roles ON DELETE CASCADE,
    allow      BIGINT NOT NULL DEFAULT 0,
    deny       BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (channel_id, role_id)
);

CREATE TABLE messages (
    id         UUID PRIMARY KEY,
    channel_id UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    author_id  UUID REFERENCES users ON DELETE SET NULL,
    content    TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at  TIMESTAMPTZ
);
CREATE INDEX messages_channel ON messages (channel_id, id DESC);
CREATE INDEX messages_created ON messages (created_at);

CREATE TABLE invites (
    code         TEXT PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    created_by   UUID REFERENCES users ON DELETE SET NULL,
    max_uses     INT CHECK (max_uses > 0),
    uses         INT         NOT NULL DEFAULT 0,
    expires_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invites_community ON invites (community_id);

CREATE TABLE audit_log (
    id           UUID PRIMARY KEY,
    community_id UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    actor_id     UUID REFERENCES users ON DELETE SET NULL,
    action       TEXT        NOT NULL,
    target_id    UUID,
    data         JSONB       NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_log_community ON audit_log (community_id, id DESC);
