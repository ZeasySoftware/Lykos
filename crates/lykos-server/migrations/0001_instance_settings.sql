-- Instanzweite Einstellungen, die zur Laufzeit änderbar sein sollen
-- (z. B. Registrierungsmodus, Limits). Statische Werte kommen aus der Umgebung.
CREATE TABLE instance_settings (
    key        TEXT PRIMARY KEY,
    value      JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO instance_settings (key, value) VALUES
    ('registration_mode', '"invite_only"');
