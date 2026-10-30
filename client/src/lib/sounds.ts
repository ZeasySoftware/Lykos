/**
 * Soundeffekte abspielen: `playSound(name)`.
 *
 * - Klänge werden per Web Audio synthetisiert (`soundSynth.ts`), Logik in `soundLogic.ts`.
 * - Der `AudioContext` entsteht erst nach der ersten Nutzerinteraktion (Autoplay-Regeln der
 *   Browser); vorher ausgelöste Töne entfallen still. Alle Fehler werden still abgefangen.
 * - Ausgabe auf dem Gerät für Hinweistöne aus „Sprache & Video“ (`setSinkId` am `AudioContext`, sonst
 *   über ein `<audio>`-Element mit `setSinkId`, sonst Systemstandard).
 * - Taubschalten (Deafen) betrifft nur den Gesprächston, nicht diese Hinweistöne – wie bei
 *   gängigen Chat-Apps: Man soll hören, dass man stumm/taub ist und dass etwas passiert.
 *   Bei „Bitte nicht stören“ bleiben Nachrichtentöne aus (siehe `messageSoundFor`).
 * - Einstellungen nur lokal (localStorage), nie auf dem Server.
 *
 * Bewusst ohne Import von `store.ts`/`voice.ts` (sonst Import-Zyklus); die Verdrahtung mit den
 * Stores steckt in `soundEvents.ts`, Nachrichten meldet `store.ts` über `playMessageSound`.
 */
import { create } from 'zustand'
import {
  MESSAGE_PRIORITY,
  MESSAGE_SOUND_INTERVAL_MS,
  SAME_SOUND_INTERVAL_MS,
  SYNC_QUIET_MS,
  createThrottle,
  messageSoundFor,
  normalizeSoundSettings,
  volumeToGain,
} from './soundLogic'
import type { SoundName, SoundSettings } from './soundLogic'
import { RECIPES, renderSound } from './soundSynth'
import { isChannelMuted } from './notify'

export type { SoundName, SoundSettings } from './soundLogic'

// ---------------------------------------------------------------------------
// Einstellungen
// ---------------------------------------------------------------------------

const KEY = 'lykos_sound_settings'

function loadSoundSettings(): SoundSettings {
  try {
    const raw = localStorage.getItem(KEY)
    return normalizeSoundSettings(raw ? JSON.parse(raw) : null)
  } catch {
    return normalizeSoundSettings(null)
  }
}

function saveSoundSettings(settings: SoundSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(settings))
  } catch {
    // Speicher nicht verfügbar – gilt dann nur für diese Sitzung.
  }
}

interface SoundSettingsStore {
  settings: SoundSettings
  setEnabled: (enabled: boolean) => void
  setVolume: (volume: number) => void
  setEvent: (name: SoundName, on: boolean) => void
  reset: () => void
}

export const useSoundSettings = create<SoundSettingsStore>()((set, get) => {
  function update(settings: SoundSettings) {
    set({ settings })
    saveSoundSettings(settings)
    if (master) master.gain.value = volumeToGain(settings.volume)
  }
  return {
    settings: loadSoundSettings(),
    setEnabled: (enabled) => update({ ...get().settings, enabled }),
    setVolume: (volume) => update(normalizeSoundSettings({ ...get().settings, volume })),
    setEvent: (name, on) => update({ ...get().settings, events: { ...get().settings.events, [name]: on } }),
    reset: () => update(normalizeSoundSettings(null)),
  }
})

// ---------------------------------------------------------------------------
// Audio-Kette: Klang → Gesamtlautstärke → Kompressor (gegen Übersteuern) → Ausgabe
// ---------------------------------------------------------------------------

type SinkContext = AudioContext & { setSinkId?: (id: string) => Promise<void> }
type SinkAudio = HTMLAudioElement & { setSinkId?: (id: string) => Promise<void> }

let ctx: SinkContext | null = null
let master: GainNode | null = null
/** Ausgang zum Gerät: `ctx.destination` oder (Fallback) ein Stream für ein `<audio>`-Element. */
let output: AudioNode | null = null
let fallbackAudio: SinkAudio | null = null
let sinkId = ''

function ensureContext(): SinkContext | null {
  if (ctx) return ctx
  try {
    const Ctor = window.AudioContext
    if (!Ctor) return null
    const c: SinkContext = new Ctor({ latencyHint: 'interactive' })
    const gain = c.createGain()
    gain.gain.value = volumeToGain(useSoundSettings.getState().settings.volume)
    const comp = c.createDynamicsCompressor()
    comp.threshold.value = -10
    comp.knee.value = 8
    comp.ratio.value = 4
    comp.attack.value = 0.002
    comp.release.value = 0.15
    gain.connect(comp)
    ctx = c
    master = gain
    output = comp
    routeOutput()
    return c
  } catch {
    return null
  }
}

/** Ausgang mit dem gewählten Gerät verbinden. */
function routeOutput() {
  const c = ctx
  const out = output
  if (!c || !out) return
  out.disconnect()
  const proto = HTMLMediaElement.prototype as SinkAudio
  if (typeof c.setSinkId === 'function' || !sinkId || typeof proto.setSinkId !== 'function') {
    // Normalfall (Chromium/WebView2 ≥ 110): Gerät direkt am AudioContext.
    fallbackAudio?.pause()
    fallbackAudio = null
    out.connect(c.destination)
    if (typeof c.setSinkId === 'function') void c.setSinkId(sinkId).catch(() => c.setSinkId?.('').catch(() => {}))
    return
  }
  // Fallback: über ein <audio>-Element, das `setSinkId` kann.
  const dest = c.createMediaStreamDestination()
  out.connect(dest)
  const audio: SinkAudio = fallbackAudio ?? new Audio()
  audio.srcObject = dest.stream
  fallbackAudio = audio
  void audio
    .setSinkId?.(sinkId)
    .catch(() => {})
    .finally(() => void audio.play().catch(() => {}))
}

/** Ausgabegerät aus den Voice-Einstellungen übernehmen (`''` = Systemstandard). */
export function setSoundOutputDevice(deviceId: string) {
  if (deviceId === sinkId) return
  sinkId = deviceId
  routeOutput()
}

// Erst nach einer Nutzerinteraktion starten (Autoplay-Regeln); `resume` deckt „suspended“ ab.
function unlock() {
  const c = ensureContext()
  if (c && c.state !== 'running') void c.resume().catch(() => {})
}
if (typeof window !== 'undefined') {
  for (const type of ['pointerdown', 'keydown'] as const) {
    window.addEventListener(type, unlock, { capture: true, passive: true })
  }
}

// ---------------------------------------------------------------------------
// Abspielen
// ---------------------------------------------------------------------------

const lastPlayed = new Map<SoundName, number>()
const messageThrottle = createThrottle(MESSAGE_SOUND_INTERVAL_MS)
let syncUntil = 0

/**
 * Klang abspielen, sofern Soundeffekte und dieses Ereignis eingeschaltet sind.
 * `preview: true` (Vorhören in den Einstellungen) ignoriert die Schalter, nicht die Lautstärke.
 */
export function playSound(name: SoundName, opts: { preview?: boolean } = {}) {
  try {
    const { settings } = useSoundSettings.getState()
    if (!opts.preview && (!settings.enabled || !settings.events[name])) return
    if (settings.volume <= 0) return
    const now = performance.now()
    if (!opts.preview && now - (lastPlayed.get(name) ?? -Infinity) < SAME_SOUND_INTERVAL_MS) return
    // Vor der ersten Interaktion gibt es noch keinen Kontext; beim Vorhören war gerade ein Klick.
    const c = opts.preview ? ensureContext() : ctx
    if (!c || !master) return
    if (c.state !== 'running') void c.resume().catch(() => {})
    lastPlayed.set(name, now)
    renderSound(c, master, RECIPES[name], c.currentTime + 0.01)
  } catch {
    // Hinweistöne dürfen nie etwas kaputt machen.
  }
}

/** Nach READY/RESYNC kurz keine Nachrichten- und Beitrittstöne (Zustand wird nur nachgeladen). */
export function markSoundSync() {
  syncUntil = performance.now() + SYNC_QUIET_MS
}

export function isSoundSyncing(): boolean {
  return performance.now() < syncUntil
}

/** Minimale Sicht auf den App-Zustand für Nachrichtentöne (vermeidet einen Import von `store.ts`). */
export interface MessageSoundState {
  user: { id: string } | null
  dmView: boolean
  selectedChannelId: string | null
  selectedDmId: string | null
  chosenStatus: string
}

/** Neue Nachricht aus dem Gateway (MESSAGE_CREATE) – spielt ggf. Nachricht/DM/Erwähnung, gedrosselt. */
export function playMessageSound(
  msg: {
    channel_id: string
    community_id: string | null
    author?: { id: string } | null
    mentions?: string[] | null
    mention_everyone?: boolean | null
  },
  s: MessageSoundState,
) {
  try {
    const me = s.user?.id
    const own = !!me && msg.author?.id === me
    const name = messageSoundFor({
      own,
      dm: msg.community_id === null,
      mentioned: !own && !!me && (!!msg.mention_everyone || (msg.mentions ?? []).includes(me)),
      channelMuted: isChannelMuted(msg.channel_id),
      dnd: s.chosenStatus === 'dnd',
      windowFocused: document.hasFocus(),
      viewedChannelId: s.dmView ? s.selectedDmId : s.selectedChannelId,
      channelId: msg.channel_id,
      syncing: isSoundSyncing(),
    })
    if (!name) return
    const { settings } = useSoundSettings.getState()
    if (!settings.enabled || !settings.events[name]) return
    if (!messageThrottle.allow(performance.now(), MESSAGE_PRIORITY[name] ?? 0)) return
    playSound(name)
  } catch {
    // still
  }
}
