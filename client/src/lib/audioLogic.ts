/**
 * Reine Logik der Ton-Einstellungen (ohne DOM/Web Audio, damit testbar und auch im AudioWorklet nutzbar):
 * Kanal-Bitrate → LiveKit-Veröffentlichungsoptionen, Lautstärken, Pegel in dB, Sprachaktivierungs-Gate.
 */

// ---------------------------------------------------------------------------
// Kanal-Ton (pro Sprachkanal, vom Server)
// ---------------------------------------------------------------------------

/** Ton-Einstellungen eines Sprachkanals, wie sie `POST /channels/{id}/voice/join` liefert. */
export interface ChannelAudio {
  /** Ziel-Bitrate in kbit/s. */
  bitrate: number
  stereo: boolean
  dtx: boolean
  red: boolean
}

export const BITRATE_MIN = 32
export const BITRATE_MAX = 256
export const BITRATE_DEFAULT = 64
/** Auswahl im Kanal-Dialog (kbit/s). */
export const BITRATE_STEPS = [32, 48, 64, 96, 128, 192, 256] as const
export const USER_LIMIT_MAX = 99

export const DEFAULT_CHANNEL_AUDIO: ChannelAudio = { bitrate: BITRATE_DEFAULT, stereo: false, dtx: true, red: true }

export function clampBitrate(kbps: number): number {
  if (!Number.isFinite(kbps)) return BITRATE_DEFAULT
  return Math.round(Math.min(BITRATE_MAX, Math.max(BITRATE_MIN, kbps)))
}

/** Veröffentlichungsoptionen der Mikrofonspur (Teilmenge von LiveKits `TrackPublishOptions`). */
export interface AudioPublishOptions {
  audioPreset: { maxBitrate: number }
  dtx: boolean
  red: boolean
  forceStereo: boolean
}

/** Kanal-Ton → LiveKit (`audioPreset.maxBitrate` in bit/s). Fehlt die Angabe, gelten die Standardwerte. */
export function audioPublishOptions(audio: Partial<ChannelAudio> | null | undefined): AudioPublishOptions {
  const a = { ...DEFAULT_CHANNEL_AUDIO, ...(audio ?? {}) }
  return {
    audioPreset: { maxBitrate: clampBitrate(a.bitrate) * 1000 },
    dtx: a.dtx,
    red: a.red,
    forceStereo: a.stereo,
  }
}

/** Anzeige einer Bitrate, z. B. „96 kbit/s“. */
export function bitrateLabel(kbps: number): string {
  return `${clampBitrate(kbps)} kbit/s`
}

// ---------------------------------------------------------------------------
// Lautstärken
// ---------------------------------------------------------------------------

/** Höchste Lautstärke pro Teilnehmer bzw. für Ein-/Ausgabe (200 %). */
export const MAX_VOLUME = 2

export function clampVolume(v: number, max = MAX_VOLUME): number {
  if (!Number.isFinite(v)) return 1
  return Math.min(max, Math.max(0, v))
}

/**
 * Wirksame Wiedergabelautstärke einer fremden Spur: lokale Teilnehmer-Lautstärke (bis 200 %) mal
 * Ausgabelautstärke (bis 200 %); taub oder lokal stummgeschaltet = 0. Bildschirmton hat keine
 * Teilnehmer-Lautstärke (`user` weglassen).
 */
export function playbackVolume(opts: {
  user?: { volume: number; muted: boolean }
  output: number
  deaf: boolean
}): number {
  if (opts.deaf || opts.user?.muted) return 0
  return clampVolume(opts.user?.volume ?? 1) * clampVolume(opts.output)
}

// ---------------------------------------------------------------------------
// Pegel & Sprachaktivierung
// ---------------------------------------------------------------------------

/** Unterste Stufe der Pegelanzeige/des Schwellwerts in dBFS. Schwellwert hier = Gate immer offen. */
export const LEVEL_FLOOR_DB = -100
export const VAD_DEFAULT_DB = -60
/** So lange bleibt das Gate nach dem letzten lauten Block offen (Wortenden nicht abschneiden). */
export const GATE_HOLD_MS = 300

/** RMS (0–1) → dBFS, nach unten auf `LEVEL_FLOOR_DB` begrenzt. */
export function rmsToDb(rms: number): number {
  if (!(rms > 0)) return LEVEL_FLOOR_DB
  return Math.max(LEVEL_FLOOR_DB, Math.min(0, 20 * Math.log10(rms)))
}

/** dBFS → Anteil 0–1 für Pegelbalken/Regler (linear in dB). */
export function dbToFraction(db: number): number {
  if (!Number.isFinite(db)) return 0
  return Math.min(1, Math.max(0, (db - LEVEL_FLOOR_DB) / -LEVEL_FLOOR_DB))
}

/** Anteil 0–1 → dBFS (Umkehrung von `dbToFraction`, auf ganze dB gerundet). */
export function fractionToDb(f: number): number {
  const clamped = Math.min(1, Math.max(0, f))
  return Math.round(LEVEL_FLOOR_DB + clamped * -LEVEL_FLOOR_DB)
}

export interface GateState {
  open: boolean
  /** Zeitpunkt (ms), zu dem der Pegel zuletzt über dem Schwellwert lag. */
  lastAbove: number
}

export const GATE_INITIAL: GateState = { open: false, lastAbove: Number.NEGATIVE_INFINITY }

/**
 * Ein Schritt des Sprachaktivierungs-Gates: offen, solange der Pegel über dem Schwellwert liegt, und
 * noch `holdMs` danach. Ein Schwellwert auf `LEVEL_FLOOR_DB` (oder darunter) lässt das Gate immer offen.
 */
export function stepGate(
  state: GateState,
  levelDb: number,
  thresholdDb: number,
  nowMs: number,
  holdMs = GATE_HOLD_MS,
): GateState {
  if (thresholdDb <= LEVEL_FLOOR_DB) return { open: true, lastAbove: nowMs }
  const lastAbove = levelDb >= thresholdDb ? nowMs : state.lastAbove
  return { open: nowMs - lastAbove <= holdMs, lastAbove }
}

// ---------------------------------------------------------------------------
// Push-to-Talk & Geräte
// ---------------------------------------------------------------------------

export const PTT_RELEASE_MAX_MS = 1000

export function clampPttDelay(ms: number): number {
  if (!Number.isFinite(ms)) return 0
  return Math.round(Math.min(PTT_RELEASE_MAX_MS, Math.max(0, ms)))
}

/** `soundOutputDeviceId`-Wert: Hinweistöne folgen dem Ausgabegerät der Sprache. */
export const SOUND_OUTPUT_FOLLOWS_VOICE = 'voice'

/** Gerät für Hinweistöne (`''` = Systemstandard). */
export function soundSinkId(s: { soundOutputDeviceId: string; outputDeviceId: string }): string {
  return s.soundOutputDeviceId === SOUND_OUTPUT_FOLLOWS_VOICE ? s.outputDeviceId : s.soundOutputDeviceId
}
