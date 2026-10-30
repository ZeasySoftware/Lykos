-- Ton-Einstellungen pro Sprachkanal (Recht `MANAGE_CHANNELS`). Bei Text-/DM-Kanälen bleiben die Standardwerte
-- stehen und haben keine Wirkung. Der Client übernimmt sie beim Beitritt in seine Veröffentlichungsoptionen
-- (`audioPreset.maxBitrate`, Opus-Stereo, DTX, RED); das Nutzerlimit prüft der Server bei `POST …/voice/join`.

ALTER TABLE channels
    -- Ziel-Bitrate der Mikrofonspur in kbit/s.
    ADD COLUMN audio_bitrate INTEGER NOT NULL DEFAULT 64 CHECK (audio_bitrate BETWEEN 32 AND 256),
    -- Opus in Stereo veröffentlichen (Musik); sonst Mono.
    ADD COLUMN audio_stereo  BOOLEAN NOT NULL DEFAULT false,
    -- Discontinuous Transmission: in Sprechpausen kaum Daten senden.
    ADD COLUMN audio_dtx     BOOLEAN NOT NULL DEFAULT true,
    -- Redundant Audio Data: Pakete doppelt senden, robuster bei Paketverlust.
    ADD COLUMN audio_red     BOOLEAN NOT NULL DEFAULT true,
    -- Höchstzahl gleichzeitiger Teilnehmer; NULL = unbegrenzt.
    ADD COLUMN user_limit    INTEGER CHECK (user_limit BETWEEN 1 AND 99);
