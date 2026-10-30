/**
 * Soundeffekte – reine Logik ohne DOM/Web Audio (Unit-Tests: `soundLogic.test.ts`).
 *
 * Hier steht, *welche* Klänge es gibt, welche standardmäßig an sind, wie Einstellungen aus dem
 * Speicher bereinigt werden und wann ein Ereignis überhaupt einen Ton auslöst (Drosselung,
 * Fokus, stummgeschaltete Kanäle, Voice-Zustandswechsel). Das Abspielen lebt in `sounds.ts`,
 * die Klangrezepte in `soundSynth.ts`.
 */

export const SOUND_NAMES = [
  'voiceJoin',
  'voiceLeave',
  'peerJoin',
  'peerLeave',
  'mute',
  'unmute',
  'deafen',
  'undeafen',
  'pttDown',
  'pttUp',
  'screenShareStart',
  'screenShareStop',
  'peerStreamStart',
  'peerStreamStop',
  'cameraOn',
  'cameraOff',
  'message',
  'mention',
  'dm',
  'friendRequest',
  'incomingCall',
  'outgoingCall',
  'connectionLost',
  'reconnected',
  'updateReady',
] as const

export type SoundName = (typeof SOUND_NAMES)[number]

export interface SoundInfo {
  label: string
  group: SoundGroup
  /** Standardmäßig eingeschaltet? */
  default: boolean
}

export type SoundGroup = 'voice' | 'calls' | 'mic' | 'media' | 'messages' | 'system'

export const SOUND_GROUPS: { id: SoundGroup; label: string }[] = [
  { id: 'voice', label: 'Sprachkanal' },
  { id: 'calls', label: 'Anrufe' },
  { id: 'mic', label: 'Mikrofon & Ton' },
  { id: 'media', label: 'Bildschirm & Kamera' },
  { id: 'messages', label: 'Nachrichten' },
  { id: 'system', label: 'Verbindung & App' },
]

export const SOUND_INFO: Record<SoundName, SoundInfo> = {
  voiceJoin: { label: 'Du trittst einem Sprachkanal bei', group: 'voice', default: true },
  voiceLeave: { label: 'Du verlässt den Sprachkanal', group: 'voice', default: true },
  peerJoin: { label: 'Jemand betritt deinen Sprachkanal', group: 'voice', default: true },
  peerLeave: { label: 'Jemand verlässt deinen Sprachkanal', group: 'voice', default: true },
  mute: { label: 'Mikrofon stumm', group: 'mic', default: true },
  unmute: { label: 'Mikrofon an', group: 'mic', default: true },
  deafen: { label: 'Ton aus (taub)', group: 'mic', default: true },
  undeafen: { label: 'Ton wieder an', group: 'mic', default: true },
  pttDown: { label: 'Push-to-Talk gedrückt', group: 'mic', default: false },
  pttUp: { label: 'Push-to-Talk losgelassen', group: 'mic', default: false },
  screenShareStart: { label: 'Deine Bildschirmübertragung startet', group: 'media', default: true },
  screenShareStop: { label: 'Deine Bildschirmübertragung endet', group: 'media', default: true },
  peerStreamStart: { label: 'Jemand im Kanal startet eine Übertragung', group: 'media', default: true },
  peerStreamStop: { label: 'Eine Übertragung im Kanal endet', group: 'media', default: true },
  cameraOn: { label: 'Deine Kamera an', group: 'media', default: true },
  cameraOff: { label: 'Deine Kamera aus', group: 'media', default: true },
  message: { label: 'Neue Nachricht', group: 'messages', default: true },
  mention: { label: 'Du wirst erwähnt', group: 'messages', default: true },
  dm: { label: 'Neue Direktnachricht', group: 'messages', default: true },
  friendRequest: { label: 'Neue Freundschaftsanfrage', group: 'messages', default: true },
  incomingCall: { label: 'Klingelton bei eingehenden Anrufen', group: 'calls', default: true },
  outgoingCall: { label: 'Freizeichen, während du anrufst', group: 'calls', default: true },
  connectionLost: { label: 'Sprachverbindung verloren', group: 'system', default: true },
  reconnected: { label: 'Sprachverbindung wiederhergestellt', group: 'system', default: true },
  updateReady: { label: 'Update bereit', group: 'system', default: true },
}

// ---------------------------------------------------------------------------
// Einstellungen
// ---------------------------------------------------------------------------

export interface SoundSettings {
  /** Hauptschalter „Soundeffekte“. */
  enabled: boolean
  /** Gesamtlautstärke 0…1. */
  volume: number
  events: Record<SoundName, boolean>
}

export const DEFAULT_VOLUME = 0.6

export function defaultSoundSettings(): SoundSettings {
  const events = {} as Record<SoundName, boolean>
  for (const name of SOUND_NAMES) events[name] = SOUND_INFO[name].default
  return { enabled: true, volume: DEFAULT_VOLUME, events }
}

/** Gespeicherte (evtl. alte oder kaputte) Einstellungen auf gültige Werte bringen. */
export function normalizeSoundSettings(raw: unknown): SoundSettings {
  const out = defaultSoundSettings()
  if (!raw || typeof raw !== 'object') return out
  const r = raw as Record<string, unknown>
  if (typeof r.enabled === 'boolean') out.enabled = r.enabled
  if (typeof r.volume === 'number' && Number.isFinite(r.volume)) out.volume = clamp01(r.volume)
  if (r.events && typeof r.events === 'object') {
    const ev = r.events as Record<string, unknown>
    for (const name of SOUND_NAMES) if (typeof ev[name] === 'boolean') out.events[name] = ev[name]
  }
  return out
}

export function clamp01(v: number): number {
  return v < 0 ? 0 : v > 1 ? 1 : v
}

/** Lautstärkeregler (linear) → Verstärkung. Quadratisch, damit der Regler sich gleichmäßig anfühlt. */
export function volumeToGain(volume: number): number {
  const v = clamp01(volume)
  return v * v
}

// ---------------------------------------------------------------------------
// Drosselung
// ---------------------------------------------------------------------------

/** Mindestabstand zwischen zwei Nachrichtentönen (Nachricht, DM, Erwähnung zusammen). */
export const MESSAGE_SOUND_INTERVAL_MS = 1500
/** Mindestabstand, bevor derselbe Klang erneut spielt (gegen Doppel-Events). */
export const SAME_SOUND_INTERVAL_MS = 120
/** Nach READY/RESYNC so lange keine Nachrichten-/Beitrittstöne. */
export const SYNC_QUIET_MS = 3000

/**
 * Drossel für eine Klanggruppe: Innerhalb von `intervalMs` nach einem Ton spielt kein weiterer –
 * außer der neue ist wichtiger (höhere Priorität, z. B. Erwähnung nach einer normalen Nachricht).
 */
export function createThrottle(intervalMs: number) {
  let last = -Infinity
  let lastPriority = -Infinity
  return {
    allow(now: number, priority = 0): boolean {
      if (now - last < intervalMs && priority <= lastPriority) return false
      last = now
      lastPriority = priority
      return true
    },
    reset() {
      last = -Infinity
      lastPriority = -Infinity
    },
  }
}

export const MESSAGE_PRIORITY: Partial<Record<SoundName, number>> = { message: 0, dm: 1, mention: 2 }

// ---------------------------------------------------------------------------
// Nachrichten
// ---------------------------------------------------------------------------

export interface MessageSoundInput {
  /** Eigene Nachricht (auch von einem anderen Gerät). */
  own: boolean
  /** Direktnachricht (Kanal ohne Community). */
  dm: boolean
  /** Erwähnt mich direkt oder per `@everyone`. */
  mentioned: boolean
  /** Kanal lokal stummgeschaltet (`notify.ts`). */
  channelMuted: boolean
  /** Eigener Status „Bitte nicht stören“. */
  dnd: boolean
  /** Lykos-Fenster hat den Fokus. */
  windowFocused: boolean
  /** Gerade geöffneter Kanal bzw. DM (oder `null`). */
  viewedChannelId: string | null
  channelId: string
  /** Initiale Synchronisation (READY/RESYNC) läuft noch. */
  syncing: boolean
}

/**
 * Welcher Ton gehört zu einer neuen Nachricht? `null` = keiner.
 *
 * - Nie für eigene Nachrichten, nie während der Synchronisation, nie bei „Bitte nicht stören“.
 * - Nur, wenn das Fenster keinen Fokus hat oder ein anderer Kanal offen ist.
 * - Stummgeschaltete Kanäle bleiben still – außer bei Erwähnungen (wie die Desktop-Benachrichtigung).
 */
export function messageSoundFor(m: MessageSoundInput): SoundName | null {
  if (m.own || m.syncing || m.dnd) return null
  if (m.windowFocused && m.viewedChannelId === m.channelId) return null
  if (m.channelMuted && !m.mentioned) return null
  if (m.mentioned) return 'mention'
  return m.dm ? 'dm' : 'message'
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

/** Ausschnitt aus dem Voice-Store (`voice.ts`), aus dem sich die Klänge ableiten. */
export interface VoiceSnapshot {
  status: 'idle' | 'connecting' | 'connected' | 'reconnecting'
  error: string | null
  selfMute: boolean
  selfDeaf: boolean
  pttPressed: boolean
  screenSharing: boolean
  cameraOn: boolean
}

/**
 * Klänge für eigene Voice-Zustandswechsel (Beitritt, Verlassen, Verbindung, Mikrofon, Ton,
 * Push-to-Talk, eigene Übertragung/Kamera).
 */
export function voiceTransitionSounds(prev: VoiceSnapshot, next: VoiceSnapshot): SoundName[] {
  const out: SoundName[] = []

  if (prev.status !== next.status) {
    if (next.status === 'connected') out.push(prev.status === 'reconnecting' ? 'reconnected' : 'voiceJoin')
    else if (next.status === 'reconnecting') out.push('connectionLost')
    else if (next.status === 'idle' && prev.status !== 'connecting') {
      // Unerwartet getrennt (Fehlertext gesetzt) → „verloren“, außer der Verlust wurde schon gemeldet.
      out.push(next.error && prev.status !== 'reconnecting' ? 'connectionLost' : 'voiceLeave')
    } else if (next.status === 'idle' && next.error) {
      // Beitritt fehlgeschlagen.
      out.push('connectionLost')
    }
  }

  // Beim Auflegen setzt `voice.ts` Übertragung/Kamera/PTT zurück – das deckt der Verlassen-Ton ab.
  if (next.status === 'idle') {
    if (prev.selfDeaf !== next.selfDeaf) out.push(next.selfDeaf ? 'deafen' : 'undeafen')
    else if (prev.selfMute !== next.selfMute) out.push(next.selfMute ? 'mute' : 'unmute')
    return out
  }

  // Entmuten hebt auch die Taubheit auf: dann zählt nur der Taub-Wechsel.
  if (prev.selfDeaf !== next.selfDeaf) out.push(next.selfDeaf ? 'deafen' : 'undeafen')
  else if (prev.selfMute !== next.selfMute) out.push(next.selfMute ? 'mute' : 'unmute')

  if (prev.pttPressed !== next.pttPressed) out.push(next.pttPressed ? 'pttDown' : 'pttUp')
  if (prev.screenSharing !== next.screenSharing) out.push(next.screenSharing ? 'screenShareStart' : 'screenShareStop')
  if (prev.cameraOn !== next.cameraOn) out.push(next.cameraOn ? 'cameraOn' : 'cameraOff')
  return out
}

/**
 * Höchstens ein Beitritts- und ein Verlassen-Ton pro Änderung, auch wenn mehrere Leute
 * gleichzeitig kommen oder gehen. `selfId` wird ignoriert.
 */
export function membershipSounds(
  prev: readonly string[],
  next: readonly string[],
  selfId: string | null | undefined,
  joined: SoundName,
  left: SoundName,
): SoundName[] {
  const before = new Set(prev.filter((id) => id !== selfId))
  const after = new Set(next.filter((id) => id !== selfId))
  const out: SoundName[] = []
  if ([...after].some((id) => !before.has(id))) out.push(joined)
  if ([...before].some((id) => !after.has(id))) out.push(left)
  return out
}
