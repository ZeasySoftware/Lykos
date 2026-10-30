-- Webcam in Voice-Kanälen: neues Recht VIDEO (1 << 13) und Kamera-Status im Voice-State.
-- Bestehende Standardrollen bekommen VIDEO dazu (wie neue Communities); sonst ändert sich nichts.
-- Idempotent formuliert, damit Tests die Migration erneut ausführen können.

UPDATE roles SET permissions = permissions | 8192 WHERE is_default;

ALTER TABLE voice_states ADD COLUMN IF NOT EXISTS video BOOLEAN NOT NULL DEFAULT false;
