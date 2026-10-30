/** Einstellungen für Sprache & Video – nur lokal (localStorage), nie auf dem Server. */
import {
  LEVEL_FLOOR_DB,
  SOUND_OUTPUT_FOLLOWS_VOICE,
  VAD_DEFAULT_DB,
  clampPttDelay,
  clampVolume,
} from './audioLogic.ts'

export type InputMode = 'voice' | 'ptt'
export type ScreenQuality = '720p30' | '1080p30' | '1080p60'

export interface VoiceSettings {
  /** Leer = Systemstandard. */
  inputDeviceId: string
  outputDeviceId: string
  videoDeviceId: string
  inputMode: InputMode
  /** `KeyboardEvent.code` der Push-to-Talk-Taste. */
  pttKey: string
  /** Anzeigename der Taste, wie beim Belegen erfasst. */
  pttKeyLabel: string
  /** Desktop: Push-to-Talk auch ohne Fensterfokus (systemweite Taste, nur während eines Gesprächs). */
  pttGlobal: boolean
  noiseSuppression: boolean
  echoCancellation: boolean
  autoGainControl: boolean
  screenQuality: ScreenQuality
  /** Eingangslautstärke (Gain vor dem Senden), 0–2 = 0–200 %. */
  inputVolume: number
  /** Ausgabelautstärke für alle Gesprächsteilnehmer, 0–2 = 0–200 %. */
  outputVolume: number
  /** Schwellwert der Sprachaktivierung in dBFS; `LEVEL_FLOOR_DB` = immer offen. */
  vadThreshold: number
  /** Push-to-Talk: Mikrofon so viele ms nach dem Loslassen noch offen lassen. */
  pttReleaseDelay: number
  /** Gerät für Hinweistöne: `'voice'` = wie Sprachausgabe, `''` = Systemstandard, sonst Geräte-ID. */
  soundOutputDeviceId: string
}

export const SCREEN_QUALITIES: Record<
  ScreenQuality,
  { label: string; width: number; height: number; fps: number; maxBitrate: number }
> = {
  '720p30': { label: '720p · 30 FPS', width: 1280, height: 720, fps: 30, maxBitrate: 2_500_000 },
  '1080p30': { label: '1080p · 30 FPS', width: 1920, height: 1080, fps: 30, maxBitrate: 4_000_000 },
  '1080p60': { label: '1080p · 60 FPS', width: 1920, height: 1080, fps: 60, maxBitrate: 6_000_000 },
}

const KEY = 'lykos_voice_settings'

export const defaultVoiceSettings: VoiceSettings = {
  inputDeviceId: '',
  outputDeviceId: '',
  videoDeviceId: '',
  inputMode: 'voice',
  pttKey: 'Backquote',
  pttKeyLabel: '^',
  pttGlobal: true,
  noiseSuppression: true,
  echoCancellation: true,
  autoGainControl: true,
  screenQuality: '1080p30',
  inputVolume: 1,
  outputVolume: 1,
  vadThreshold: VAD_DEFAULT_DB,
  pttReleaseDelay: 0,
  soundOutputDeviceId: SOUND_OUTPUT_FOLLOWS_VOICE,
}

/** Gespeicherte Werte übernehmen, Zahlen auf gültige Bereiche begrenzen. */
export function normalizeVoiceSettings(raw: Partial<VoiceSettings> | null | undefined): VoiceSettings {
  const s = { ...defaultVoiceSettings, ...(raw ?? {}) }
  const num = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) ? v : fallback)
  return {
    ...s,
    inputVolume: clampVolume(num(s.inputVolume, 1)),
    outputVolume: clampVolume(num(s.outputVolume, 1)),
    vadThreshold: Math.round(Math.min(0, Math.max(LEVEL_FLOOR_DB, num(s.vadThreshold, VAD_DEFAULT_DB)))),
    pttReleaseDelay: clampPttDelay(num(s.pttReleaseDelay, 0)),
    soundOutputDeviceId: typeof s.soundOutputDeviceId === 'string' ? s.soundOutputDeviceId : SOUND_OUTPUT_FOLLOWS_VOICE,
  }
}

export function loadVoiceSettings(): VoiceSettings {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return normalizeVoiceSettings(JSON.parse(raw) as Partial<VoiceSettings>)
  } catch {
    // Kaputter oder fehlender Speicher – Standardwerte.
  }
  return { ...defaultVoiceSettings }
}

export function saveVoiceSettings(settings: VoiceSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(settings))
  } catch {
    // Speicher nicht verfügbar – gilt dann nur für diese Sitzung.
  }
}
