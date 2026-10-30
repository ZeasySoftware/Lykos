-- Paket B: Voice-Status. Quelle der Wahrheit sind die LiveKit-Webhooks; Mute/Deaf meldet der Client.
-- Ein Nutzer ist höchstens in einem Voice-Kanal (PK user_id). Kanal-/Community-Löschung räumt per Kaskade auf.

CREATE TABLE voice_states (
    user_id         UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    channel_id      UUID        NOT NULL REFERENCES channels ON DELETE CASCADE,
    community_id    UUID        NOT NULL REFERENCES communities ON DELETE CASCADE,
    -- LiveKit-Participant-SID: verspätete Events einer älteren Verbindung werden ignoriert.
    participant_sid TEXT        NOT NULL DEFAULT '',
    self_mute       BOOLEAN     NOT NULL DEFAULT false,
    self_deaf       BOOLEAN     NOT NULL DEFAULT false,
    streaming       BOOLEAN     NOT NULL DEFAULT false,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX voice_states_channel ON voice_states (channel_id);
CREATE INDEX voice_states_community ON voice_states (community_id);
