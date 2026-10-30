-- Paket G + H: Eine Tabelle für alle Passwort-Reset-Links.
--
-- `password_reset_tokens` (Paket H, 0013) nimmt jetzt auch die Links aus „Passwort vergessen“ auf
-- (`kind = 'email_reset'`, 30 min). Admin-Links (`admin_reset`) gelten 24 h. Eingelöst werden beide über
-- `POST /auth/password-reset/verify|complete`. Die Tabelle `password_resets` aus 0012 entfällt; offene
-- Links daraus werden übernommen (Hash und Ablauf bleiben gleich, die Links funktionieren also weiter).

ALTER TABLE password_reset_tokens DROP CONSTRAINT IF EXISTS password_reset_tokens_kind_check;
ALTER TABLE password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_kind_check CHECK (kind IN ('admin_reset', 'email_reset'));

INSERT INTO password_reset_tokens (id, user_id, token_hash, kind, created_by, expires_at, created_at)
SELECT gen_random_uuid(), r.user_id, r.token_hash, 'email_reset', NULL, r.expires_at, r.created_at
FROM password_resets r
WHERE r.expires_at > now()
ON CONFLICT (token_hash) DO NOTHING;

DROP TABLE password_resets;
