/**
 * Voice & Screenshare über LiveKit.
 *
 * Hält höchstens eine Raum-Verbindung (= ein Voice-Kanal oder ein Anruf in einer DM, dann ist
 * `communityId` `null`). Der sichtbare Voice-Status aller
 * Nutzer kommt vom Server (Webhooks → Gateway, siehe `store.ts`); hier lebt nur der lokale
 * Zustand: eigene Verbindung, Mikrofon, Sprechanzeige, Bildschirmübertragungen.
 *
 * Mikrofon-Spuren werden immer abonniert, Bildschirmübertragungen nur, solange man zuschaut,
 * Kameras nur, solange eine sichtbare Kachel sie anfordert (`requestCamera`, `autoSubscribe: false`)
 * – spart Bandbreite, und adaptiveStream wählt die Simulcast-Ebene nach der Kachelgröße.
 */
import { create } from 'zustand'
// livekit-client ist groß und wird erst beim ersten Beitritt geladen (kleineres Start-Bundle).
import type {
  AudioCaptureOptions,
  ConnectionQuality,
  DisconnectReason,
  Participant,
  RemoteAudioTrack,
  RemoteTrack,
  Room,
  Track,
  VideoCaptureOptions,
} from 'livekit-client'
import { ApiError, endpoints } from './api'
import { useCalls } from './callStore'
import { DEFAULT_CHANNEL_AUDIO, MAX_VOLUME, audioPublishOptions, clampPttDelay, playbackVolume } from './audioLogic'
import type { ChannelAudio } from './audioLogic'
import { errorText } from './errors'
import { createMicProcessor, createOutputContext } from './micChain'
import type { MicProcessor, MicProcessorSettings } from './micChain'
import { Permission, has } from './permissions'
import { SCREEN_QUALITIES, loadVoiceSettings, saveVoiceSettings } from './voiceSettings'
import type { ScreenQuality, VoiceSettings } from './voiceSettings'
import { useStore } from '../store'

export type VoiceStatus = 'idle' | 'connecting' | 'connected' | 'reconnecting'
/** Werte wie LiveKits `ConnectionQuality`. */
export type VoiceQuality = 'excellent' | 'good' | 'poor' | 'lost' | 'unknown'

type LiveKit = typeof import('livekit-client')
let lk: LiveKit | null = null
function loadLiveKit(): Promise<LiveKit> {
  return import('livekit-client').then((m) => (lk = m))
}

/** Raum, dem man beitritt: Sprachkanal einer Community oder DM-Kanal (`community_id: null`, Anruf). */
export interface CallRoomRef {
  id: string
  community_id: string | null
}

interface VoiceStore {
  status: VoiceStatus
  channelId: string | null
  /** `null` bei Anrufen in DMs. */
  communityId: string | null
  error: string | null
  selfMute: boolean
  selfDeaf: boolean
  /** Push-to-Talk-Taste gerade gedrückt. */
  pttPressed: boolean
  canSpeak: boolean
  canStream: boolean
  /** Recht `VIDEO` im aktuellen Kanal. */
  canVideo: boolean
  /** Eigene Kamera ist an. */
  cameraOn: boolean
  /** Identities mit eingeschalteter Kamera im eigenen Raum (inkl. einem selbst). */
  cameras: string[]
  /** Identities aller Teilnehmer im eigenen Raum (inkl. einem selbst). */
  participants: string[]
  /** Fokussierte Kachel der Call-Bühne (`tileKey`), `null` = Raster. */
  focus: string | null
  quality: VoiceQuality
  /** Verbindungsqualität der anderen Teilnehmer (Identity → Qualität). */
  qualities: Record<string, VoiceQuality>
  /** Identities (= Nutzer-IDs), die gerade sprechen. */
  speaking: string[]
  /** Wer zuletzt gesprochen hat (ohne einen selbst) – für die Mini-Ansicht. */
  lastSpeaker: string | null
  /** Identities mit aktiver Bildschirmübertragung im eigenen Raum (inkl. einem selbst). */
  streams: string[]
  /** Streams, die im Hauptbereich angezeigt werden. */
  watching: string[]
  screenSharing: boolean
  settings: VoiceSettings
  /** Wird bei Track-Änderungen hochgezählt, damit Video-Kacheln neu anhängen. */
  trackVersion: number
  /** Lokale Lautstärke/Stummschaltung anderer Teilnehmer (nur auf diesem Gerät, user_id → Einstellung). */
  userAudio: Record<string, UserAudio>

  /** Beitreten; `video: true` schaltet nach dem Verbinden gleich die Kamera ein (Annehmen mit Video). */
  join: (channel: CallRoomRef, opts?: { video?: boolean }) => Promise<void>
  leave: () => Promise<void>
  toggleMute: () => void
  toggleDeaf: () => void
  startScreenShare: (quality: ScreenQuality) => Promise<void>
  stopScreenShare: () => Promise<void>
  toggleCamera: () => Promise<void>
  setFocus: (key: string | null) => void
  watch: (userId: string) => void
  unwatch: (userId: string) => void
  unwatchAll: () => void
  updateSettings: (patch: Partial<VoiceSettings>) => void
  /** Lokale Lautstärke eines Teilnehmers (0–2 = bis 200 %), wirkt sofort auf seine Mikrofon-Spur. */
  setUserVolume: (userId: string, volume: number) => void
  /** Teilnehmer nur für sich selbst stummschalten. */
  setUserMuted: (userId: string, muted: boolean) => void
  clearError: () => void
  /** Intern: PTT-Taste gedrückt/losgelassen. */
  setPttPressed: (pressed: boolean) => void
  /** Intern: Mikrofon an Rechte/Einstellungen angleichen. */
  refreshMic: () => void
  /** Intern: eigenen Status beim Server korrigieren, falls er abweicht. */
  checkServerState: () => void
  /**
   * Intern: Sprechen/Bildschirm/Kamera neu bestimmen (Kanalrechte aus dem App-Store und die
   * Rechte, die LiveKit dem Teilnehmer gerade gewährt). Entzogenes wird sofort beendet.
   */
  applyCapabilities: () => void
}

export interface UserAudio {
  volume: number
  muted: boolean
}

const USER_AUDIO_KEY = 'lykos_user_audio'
export const DEFAULT_USER_AUDIO: UserAudio = { volume: 1, muted: false }

function loadUserAudio(): Record<string, UserAudio> {
  try {
    return JSON.parse(localStorage.getItem(USER_AUDIO_KEY) ?? '{}') as Record<string, UserAudio>
  } catch {
    return {}
  }
}

interface Capabilities {
  canSpeak: boolean
  canStream: boolean
  canVideo: boolean
}

/**
 * Rechte laut App-Store (Kanalrechte); `null`, wenn der Kanal nicht (mehr) bekannt ist. In DM-Anrufen darf
 * jeder Teilnehmer alles (wie auf dem Server).
 */
function channelCapabilities(communityId: string | null, channelId: string): Capabilities | null {
  if (communityId === null) {
    return useStore.getState().dms[channelId] ? { canSpeak: true, canStream: true, canVideo: true } : null
  }
  const detail = useStore.getState().details[communityId]
  if (!detail?.channels.some((c) => c.id === channelId)) return null
  const perms = detail.channel_permissions[channelId] ?? detail.permissions
  return {
    canSpeak: has(perms, Permission.SPEAK),
    canStream: has(perms, Permission.STREAM),
    canVideo: has(perms, Permission.VIDEO),
  }
}

/**
 * Rechte, die LiveKit dem eigenen Teilnehmer gerade gewährt. Der Server gleicht sie bei
 * Rechteänderungen im laufenden Call an (`UpdateParticipant`). `null` = (noch) unbekannt.
 */
function livekitCapabilities(r: Room | null): Capabilities | null {
  const p = r?.localParticipant.permissions
  if (!p) return null
  if (!p.canPublish) return { canSpeak: false, canStream: false, canVideo: false }
  // Leere Liste heißt bei LiveKit „alle Quellen“. Werte wie `TrackSource` im Protokoll.
  const sources = p.canPublishSources as number[]
  const allows = (source: number) => sources.length === 0 || sources.includes(source)
  return { canSpeak: allows(2), canStream: allows(3), canVideo: allows(1) }
}

let room: Room | null = null
/**
 * Wiedergabe läuft über Web Audio (`webAudioMix`), damit Lautstärken über 100 % gehen. Den Kontext
 * legen wir selbst an (Ausgabegerät per `setSinkId`) und schließen ihn beim Verlassen.
 */
let playbackCtx: AudioContext | null = null
/** Prozessor der eigenen Mikrofonspur (Eingangslautstärke + Sprachaktivierung). */
let micProcessor: MicProcessor | null = null
/** Ton-Einstellungen des aktuellen Kanals (vom Server beim Beitritt). */
let channelAudio: ChannelAudio = DEFAULT_CHANNEL_AUDIO
/** Push-to-Talk: verzögertes Loslassen. */
let pttReleaseTimer: ReturnType<typeof setTimeout> | null = null
/** Kamera-Anforderungen sichtbarer Kacheln je Identity (Zähler, siehe `requestCamera`). */
const cameraWants = new Map<string, number>()
/** Abos neu abgleichen – wird gesetzt, sobald der Store steht. */
let resync: () => void = () => {}
/** Erhöht sich bei jedem Beitritt/Verlassen; veraltete asynchrone Schritte brechen ab. */
let joinSeq = 0
/** Mikrofon-Änderungen nacheinander ausführen (PTT kann schnell hintereinander feuern). */
let micQueue: Promise<void> = Promise.resolve()
/** Während die PTT-Taste in den Einstellungen belegt wird, ignorieren wir sie. */
let capturingKey = false

const audioContainer = document.createElement('div')
audioContainer.hidden = true
document.body.appendChild(audioContainer)

/** Kamera: 720p30, Simulcast-Ebenen 180p/360p (siehe `toggleCamera`). */
function cameraOptions(s: VoiceSettings): VideoCaptureOptions {
  return {
    deviceId: s.videoDeviceId || undefined,
    resolution: { width: 1280, height: 720, frameRate: 30 },
  }
}

function sameList(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i])
}

function captureOptions(s: VoiceSettings): AudioCaptureOptions {
  return {
    deviceId: s.inputDeviceId || undefined,
    noiseSuppression: s.noiseSuppression,
    echoCancellation: s.echoCancellation,
    autoGainControl: s.autoGainControl,
    // Stereo-Kanäle (Musik): zwei Kanäle aufnehmen, falls das Gerät sie hat.
    ...(channelAudio.stereo ? { channelCount: 2 } : {}),
  }
}

function micProcessorSettings(s: VoiceSettings): MicProcessorSettings {
  return {
    gain: s.inputVolume,
    // Bei Push-to-Talk schaltet die Taste, das Gate bleibt offen.
    gateEnabled: s.inputMode === 'voice',
    threshold: s.vadThreshold,
    stereo: channelAudio.stereo,
  }
}

/** DM-Anruf: Klingeln abbrechen, falls man selbst anruft und noch niemand angenommen hat (Server prüft das). */
function cancelRinging(channelId: string) {
  endpoints.callDecline(channelId).catch(() => {})
}

export const useVoice = create<VoiceStore>()((set, get) => {
  function micShouldBeOn(): boolean {
    const s = get()
    return s.canSpeak && !s.selfMute && !s.selfDeaf && (s.settings.inputMode === 'voice' || s.pttPressed)
  }

  function applyMic() {
    micQueue = micQueue.then(async () => {
      const r = room
      if (!r || !get().canSpeak || get().status === 'idle') return
      const on = micShouldBeOn()
      const pub = r.localParticipant.getTrackPublication(lk!.Track.Source.Microphone)
      // Schon im gewünschten Zustand? (Spur existiert und ist (un)gemutet.)
      if (pub?.track && pub.isMuted === !on) return
      try {
        // Beim ersten Mal Spur anlegen (fragt nach Mikrofon-Zugriff), danach nur (ent)muten.
        if (!pub) {
          await r.localParticipant.setMicrophoneEnabled(
            true,
            captureOptions(get().settings),
            audioPublishOptions(channelAudio),
          )
          await attachMicProcessor(r)
        }
        await r.localParticipant.setMicrophoneEnabled(micShouldBeOn())
      } catch {
        set({ error: 'Kein Zugriff auf das Mikrofon.', selfMute: true })
        reportState()
      }
    })
  }

  /** Eingangslautstärke und Sprachaktivierung vor dem Senden (Web Audio). Ohne Erfolg: Spur bleibt roh. */
  async function attachMicProcessor(r: Room) {
    const track = r.localParticipant.getTrackPublication(lk!.Track.Source.Microphone)?.audioTrack
    if (!track || room !== r) return
    const processor = createMicProcessor(micProcessorSettings(get().settings))
    try {
      await track.setProcessor(processor)
      micProcessor = processor
    } catch {
      micProcessor = null
    }
  }

  /** Lautstärke aller fremden Audiospuren neu setzen (Teilnehmer-Lautstärke × Ausgabe, taub = 0). */
  function applyRemoteVolumes() {
    const r = room
    if (!r || !lk) return
    const s = get()
    for (const p of r.remoteParticipants.values()) {
      for (const pub of p.audioTrackPublications.values()) {
        const track = pub.audioTrack as RemoteAudioTrack | undefined
        if (!track) continue
        const mic = pub.source === lk.Track.Source.Microphone
        track.setVolume(
          playbackVolume({
            user: mic ? s.userAudio[p.identity] : undefined,
            output: s.settings.outputVolume,
            deaf: s.selfDeaf,
          }),
        )
      }
    }
  }

  function setUserAudio(userId: string, patch: Partial<UserAudio>) {
    const prev = get().userAudio[userId] ?? DEFAULT_USER_AUDIO
    const next = { ...prev, ...patch }
    const userAudio = { ...get().userAudio }
    // Standardwerte nicht speichern.
    if (next.volume === 1 && !next.muted) delete userAudio[userId]
    else userAudio[userId] = next
    set({ userAudio })
    try {
      localStorage.setItem(USER_AUDIO_KEY, JSON.stringify(userAudio))
    } catch {
      // egal
    }
    applyRemoteVolumes()
  }

  /** Erwarteter eigener Status, wie ihn andere sehen sollen. */
  function expectedState() {
    const s = get()
    return { self_mute: s.selfMute || s.selfDeaf || !s.canSpeak, self_deaf: s.selfDeaf }
  }

  let reporting = false
  function reportState() {
    if (get().status === 'idle') return
    reporting = true
    endpoints
      .setVoiceState(expectedState())
      .catch(() => {})
      .finally(() => {
        reporting = false
        // Der Webhook kann nach unserer Meldung eingetroffen sein – dann nachziehen.
        get().checkServerState()
      })
  }

  function teardown(error: string | null = null) {
    joinSeq += 1
    const r = room
    room = null
    r?.removeAllListeners()
    audioContainer.replaceChildren()
    micProcessor = null
    if (pttReleaseTimer) clearTimeout(pttReleaseTimer)
    pttReleaseTimer = null
    const ctx = playbackCtx
    playbackCtx = null
    // Erst nach dem Trennen schließen (LiveKit baut seine Knoten noch ab).
    if (ctx) setTimeout(() => void ctx.close().catch(() => {}), 1000)
    set({
      status: 'idle',
      channelId: null,
      communityId: null,
      pttPressed: false,
      speaking: [],
      streams: [],
      watching: [],
      screenSharing: false,
      cameraOn: false,
      cameras: [],
      participants: [],
      focus: null,
      quality: 'unknown',
      qualities: {},
      lastSpeaker: null,
      trackVersion: get().trackVersion + 1,
      ...(error ? { error } : {}),
    })
  }

  function updateStreams() {
    const r = room
    if (!r) return
    const streams: string[] = []
    const cameras: string[] = []
    const participants: string[] = []
    for (const p of [r.localParticipant, ...r.remoteParticipants.values()]) {
      participants.push(p.identity)
      if (p.getTrackPublication(lk!.Track.Source.ScreenShare)) streams.push(p.identity)
      const cam = p.getTrackPublication(lk!.Track.Source.Camera)
      if (cam && !cam.isMuted) cameras.push(p.identity)
    }
    set((s) => ({
      streams,
      cameras,
      participants: sameList(s.participants, participants) ? s.participants : participants,
      watching: s.watching.filter((id) => streams.includes(id)),
      screenSharing: streams.includes(r.localParticipant.identity),
      cameraOn: cameras.includes(r.localParticipant.identity),
    }))
  }

  /** Mikrofone immer, Bildschirm (+Ton) nur bei angesehenen Streams, Kameras nur für sichtbare Kacheln. */
  function syncSubscriptions() {
    const r = room
    if (!r) return
    updateStreams()
    const watching = new Set(get().watching)
    for (const p of r.remoteParticipants.values()) {
      for (const pub of p.trackPublications.values()) {
        const screen = pub.source === lk!.Track.Source.ScreenShare || pub.source === lk!.Track.Source.ScreenShareAudio
        const camera = pub.source === lk!.Track.Source.Camera
        const want =
          pub.source === lk!.Track.Source.Microphone ||
          (screen && watching.has(p.identity)) ||
          (camera && (cameraWants.get(p.identity) ?? 0) > 0)
        if (pub.isSubscribed !== want) pub.setSubscribed(want)
      }
    }
  }

  function bindRoom(r: Room, { RoomEvent, DisconnectReason: Reason }: LiveKit) {
    const mine = () => room === r
    r.on(RoomEvent.Disconnected, (reason?: DisconnectReason) => {
      if (!mine()) return
      const text =
        reason === Reason.PARTICIPANT_REMOVED
          ? 'Du wurdest aus dem Sprachkanal entfernt.'
          : reason === Reason.ROOM_DELETED
            ? 'Der Sprachkanal wurde geschlossen.'
            : reason === Reason.DUPLICATE_IDENTITY
              ? 'Du bist mit einem anderen Gerät beigetreten.'
              : reason === Reason.CLIENT_INITIATED || reason === undefined
                ? null
                : // Noch nie verbunden (z. B. LiveKit nicht erreichbar): nichts „verloren“.
                  get().status === 'connecting'
                  ? 'Verbindung zum Sprachserver fehlgeschlagen.'
                  : 'Verbindung zum Sprachkanal verloren.'
      // DM-Anruf ungewollt beendet (nie verbunden, Verbindung weg): Klingeln abbrechen, falls man allein anrief.
      const { communityId, channelId } = get()
      if (text && communityId === null && channelId) cancelRinging(channelId)
      teardown(text)
    })
      .on(RoomEvent.Reconnecting, () => mine() && set({ status: 'reconnecting' }))
      .on(RoomEvent.Reconnected, () => {
        if (!mine()) return
        set({ status: 'connected' })
        syncSubscriptions()
      })
      .on(RoomEvent.ActiveSpeakersChanged, (speakers: Participant[]) => {
        if (!mine()) return
        const remote = speakers.find((p) => p.identity !== r.localParticipant.identity)
        set((s) => ({
          speaking: speakers.map((p) => p.identity),
          lastSpeaker: remote ? remote.identity : s.lastSpeaker,
        }))
      })
      .on(RoomEvent.ConnectionQualityChanged, (quality: ConnectionQuality, p: Participant) => {
        if (!mine()) return
        if (p.identity === r.localParticipant.identity) set({ quality: quality as VoiceQuality })
        else set((s) => ({ qualities: { ...s.qualities, [p.identity]: quality as VoiceQuality } }))
      })
      .on(RoomEvent.ParticipantPermissionsChanged, (_prev: unknown, p: Participant) => {
        // Server hat die eigenen Rechte im laufenden Call geändert (z. B. STREAM entzogen).
        if (mine() && p.identity === r.localParticipant.identity) get().applyCapabilities()
      })
      .on(RoomEvent.ParticipantConnected, () => mine() && syncSubscriptions())
      .on(RoomEvent.ParticipantDisconnected, () => mine() && syncSubscriptions())
      .on(RoomEvent.TrackPublished, () => mine() && syncSubscriptions())
      .on(RoomEvent.TrackUnpublished, () => mine() && syncSubscriptions())
      .on(RoomEvent.TrackMuted, () => {
        if (!mine()) return
        updateStreams()
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
      .on(RoomEvent.TrackUnmuted, () => {
        if (!mine()) return
        updateStreams()
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
      .on(RoomEvent.LocalTrackPublished, () => {
        if (!mine()) return
        updateStreams()
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
      .on(RoomEvent.LocalTrackUnpublished, () => {
        if (!mine()) return
        updateStreams()
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
      .on(RoomEvent.TrackSubscribed, (track: RemoteTrack, _pub: unknown, participant: Participant) => {
        if (!mine()) return
        if (track.kind === lk!.Track.Kind.Audio) {
          // Wiedergabe über Web Audio (`webAudioMix`), das Element selbst bleibt stumm.
          audioContainer.appendChild(track.attach())
          // Lokale Lautstärke des Teilnehmers (nur Mikrofon) × Ausgabelautstärke, taub = 0.
          const s = get()
          ;(track as RemoteAudioTrack).setVolume(
            playbackVolume({
              user: track.source === lk!.Track.Source.Microphone ? s.userAudio[participant.identity] : undefined,
              output: s.settings.outputVolume,
              deaf: s.selfDeaf,
            }),
          )
        }
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
      .on(RoomEvent.TrackUnsubscribed, (track: RemoteTrack) => {
        if (!mine()) return
        if (track.kind === lk!.Track.Kind.Audio) for (const el of track.detach()) el.remove()
        set((s) => ({ trackVersion: s.trackVersion + 1 }))
      })
  }

  resync = syncSubscriptions

  return {
    status: 'idle',
    channelId: null,
    communityId: null,
    error: null,
    selfMute: false,
    selfDeaf: false,
    pttPressed: false,
    canSpeak: false,
    canStream: false,
    canVideo: false,
    cameraOn: false,
    cameras: [],
    participants: [],
    focus: null,
    quality: 'unknown',
    qualities: {},
    speaking: [],
    lastSpeaker: null,
    streams: [],
    watching: [],
    screenSharing: false,
    settings: loadVoiceSettings(),
    trackVersion: 0,
    userAudio: loadUserAudio(),

    async join(channel, opts = {}) {
      const s = get()
      if (s.channelId === channel.id && s.status !== 'idle') return
      if (s.status !== 'idle') await get().leave()

      const seq = ++joinSeq
      const caps = channelCapabilities(channel.community_id, channel.id) ?? {
        canSpeak: false,
        canStream: false,
        canVideo: false,
      }
      set({
        status: 'connecting',
        channelId: channel.id,
        communityId: channel.community_id,
        error: null,
        ...caps,
        speaking: [],
        streams: [],
        watching: [],
        cameraOn: false,
        cameras: [],
        participants: [],
        focus: null,
        quality: 'unknown',
        qualities: {},
        lastSpeaker: null,
      })

      let r: Room | null = null
      try {
        const [{ url, token, audio }, livekit] = await Promise.all([
          endpoints.voiceJoin(channel.id),
          loadLiveKit(),
        ])
        if (seq !== joinSeq) return
        channelAudio = { ...DEFAULT_CHANNEL_AUDIO, ...(audio ?? {}) }
        const settings = get().settings
        playbackCtx = createOutputContext(settings.outputDeviceId)
        r = new livekit.Room({
          adaptiveStream: true,
          dynacast: true,
          audioCaptureDefaults: captureOptions(settings),
          videoCaptureDefaults: cameraOptions(settings),
          audioOutput: settings.outputDeviceId ? { deviceId: settings.outputDeviceId } : undefined,
          webAudioMix: { audioContext: playbackCtx },
          publishDefaults: { ...audioPublishOptions(channelAudio), simulcast: true },
        })
        room = r
        bindRoom(r, livekit)
        await r.connect(url, token, { autoSubscribe: false })
        if (seq !== joinSeq) {
          void r.disconnect()
          return
        }
        set({ status: 'connected' })
        syncSubscriptions()
        // Die Rechte im Token können enger sein als die zuletzt geladenen Kanalrechte.
        get().applyCapabilities()
        if (get().canSpeak) applyMic()
        reportState()
        if (opts.video && get().canVideo) void get().toggleCamera()
      } catch (e) {
        if (seq !== joinSeq) return
        void r?.disconnect()
        // Anruf in einer DM nicht zustande gekommen: Klingeln bei den anderen abbrechen.
        if (channel.community_id === null && !(e instanceof ApiError)) cancelRinging(channel.id)
        teardown(e instanceof ApiError ? errorText(e) : 'Verbindung zum Sprachserver fehlgeschlagen.')
      }
    },

    async leave() {
      const r = room
      const { communityId, channelId, status } = get()
      // DM-Anruf: Wer allein auflegt, solange es noch klingelt, bricht das Klingeln ab (sonst wirkungslos).
      if (status !== 'idle' && communityId === null && channelId) cancelRinging(channelId)
      teardown()
      await r?.disconnect()
    },

    toggleMute() {
      const s = get()
      // Entmuten hebt auch Taubheit auf (wie gewohnt).
      if (s.selfDeaf) set({ selfDeaf: false, selfMute: false })
      else set({ selfMute: !s.selfMute })
      applyRemoteVolumes()
      applyMic()
      reportState()
    },

    toggleDeaf() {
      set({ selfDeaf: !get().selfDeaf })
      applyRemoteVolumes()
      applyMic()
      reportState()
    },

    async startScreenShare(quality) {
      const r = room
      if (!r || !get().canStream) return
      get().updateSettings({ screenQuality: quality })
      const q = SCREEN_QUALITIES[quality]
      try {
        await r.localParticipant.setScreenShareEnabled(
          true,
          {
            audio: true,
            systemAudio: 'include',
            selfBrowserSurface: 'exclude',
            resolution: { width: q.width, height: q.height, frameRate: q.fps },
            contentHint: q.fps >= 60 ? 'motion' : 'detail',
          },
          {
            simulcast: true,
            screenShareEncoding: { maxBitrate: q.maxBitrate, maxFramerate: q.fps },
            screenShareSimulcastLayers:
              q.height > 720
                ? [lk!.ScreenSharePresets.h360fps15, lk!.ScreenSharePresets.h720fps30]
                : [lk!.ScreenSharePresets.h360fps15],
          },
        )
      } catch (e) {
        // Abbruch im Auswahldialog ist kein Fehler.
        if (e instanceof Error && e.name === 'NotAllowedError') return
        set({ error: 'Bildschirmübertragung konnte nicht gestartet werden.' })
      }
      updateStreams()
    },

    async stopScreenShare() {
      await room?.localParticipant.setScreenShareEnabled(false)
      updateStreams()
    },

    async toggleCamera() {
      const r = room
      if (!r || get().status !== 'connected') return
      const pub = r.localParticipant.getTrackPublication(lk!.Track.Source.Camera)
      if (pub?.track) {
        // Ganz unveröffentlichen statt nur muten: Kameralicht geht aus, der Server sieht `track_unpublished`.
        await r.localParticipant.unpublishTrack(pub.track, true).catch(() => {})
        updateStreams()
        return
      }
      if (!get().canVideo) return
      try {
        await r.localParticipant.setCameraEnabled(true, cameraOptions(get().settings), {
          simulcast: true,
          videoEncoding: lk!.VideoPresets.h720.encoding,
          videoSimulcastLayers: [lk!.VideoPresets.h180, lk!.VideoPresets.h360],
        })
      } catch (e) {
        const name = e instanceof Error ? e.name : ''
        set({
          error:
            name === 'NotAllowedError' || name === 'SecurityError'
              ? 'Kein Zugriff auf die Kamera – bitte in den Browser- bzw. Systemeinstellungen erlauben.'
              : name === 'NotFoundError' || name === 'OverconstrainedError'
                ? 'Keine Kamera gefunden.'
                : name === 'NotReadableError'
                  ? 'Die Kamera wird gerade von einem anderen Programm benutzt.'
                  : 'Kamera konnte nicht gestartet werden.',
        })
      }
      updateStreams()
    },

    setFocus(key) {
      if (get().focus !== key) set({ focus: key })
    },

    watch(userId) {
      if (!get().watching.includes(userId)) set((s) => ({ watching: [...s.watching, userId] }))
      syncSubscriptions()
    },

    unwatch(userId) {
      set((s) => ({ watching: s.watching.filter((id) => id !== userId) }))
      syncSubscriptions()
    },

    unwatchAll() {
      set({ watching: [] })
      syncSubscriptions()
    },

    updateSettings(patch) {
      const prev = get().settings
      const settings = { ...prev, ...patch }
      const pttChanged =
        settings.inputMode !== prev.inputMode ||
        settings.pttKey !== prev.pttKey ||
        settings.pttGlobal !== prev.pttGlobal
      if (pttChanged && pttReleaseTimer) {
        clearTimeout(pttReleaseTimer)
        pttReleaseTimer = null
      }
      set({ settings, ...(pttChanged ? { pttPressed: false } : {}) })
      saveVoiceSettings(settings)
      const r = room
      if (!r) return
      if (settings.outputVolume !== prev.outputVolume) applyRemoteVolumes()
      if (
        settings.inputVolume !== prev.inputVolume ||
        settings.vadThreshold !== prev.vadThreshold ||
        settings.inputMode !== prev.inputMode
      ) {
        micProcessor?.update(micProcessorSettings(settings))
      }
      if (patch.inputDeviceId !== undefined && patch.inputDeviceId !== prev.inputDeviceId) {
        void r.switchActiveDevice('audioinput', settings.inputDeviceId || 'default').catch(() => {})
      }
      if (patch.outputDeviceId !== undefined && patch.outputDeviceId !== prev.outputDeviceId) {
        void r.switchActiveDevice('audiooutput', settings.outputDeviceId || 'default').catch(() => {})
      }
      if (patch.videoDeviceId !== undefined && patch.videoDeviceId !== prev.videoDeviceId) {
        const cam = r.localParticipant.getTrackPublication(lk!.Track.Source.Camera)?.videoTrack
        void cam?.restartTrack(cameraOptions(settings)).catch(() => {})
      }
      const constraintsChanged = (['noiseSuppression', 'echoCancellation', 'autoGainControl'] as const).some(
        (k) => patch[k] !== undefined && patch[k] !== prev[k],
      )
      if (constraintsChanged) {
        const track = r.localParticipant.getTrackPublication(lk!.Track.Source.Microphone)?.audioTrack
        void track?.restartTrack(captureOptions(settings)).catch(() => {})
      }
      applyMic()
    },

    setUserVolume(userId, volume) {
      setUserAudio(userId, { volume: Math.max(0, Math.min(MAX_VOLUME, volume)) })
    },

    setUserMuted(userId, muted) {
      setUserAudio(userId, { muted })
    },

    clearError() {
      set({ error: null })
    },

    setPttPressed(pressed) {
      if (pttReleaseTimer) {
        clearTimeout(pttReleaseTimer)
        pttReleaseTimer = null
      }
      if (get().pttPressed === pressed) return
      const delay = pressed ? 0 : clampPttDelay(get().settings.pttReleaseDelay)
      if (delay > 0) {
        // Mikrofon kurz offen lassen, damit Wortenden nicht abgeschnitten werden.
        pttReleaseTimer = setTimeout(() => {
          pttReleaseTimer = null
          set({ pttPressed: false })
          applyMic()
        }, delay)
        return
      }
      set({ pttPressed: pressed })
      applyMic()
    },

    refreshMic() {
      applyMic()
      reportState()
    },

    checkServerState() {
      const s = get()
      if (s.status !== 'connected' || !s.channelId || reporting) return
      const app = useStore.getState()
      const states = s.communityId
        ? app.voiceStates[s.communityId]
        : useCalls.getState().calls[s.channelId]?.voiceStates
      const own = states?.find((v) => v.user_id === app.user?.id && v.channel_id === s.channelId)
      if (!own) return
      const exp = expectedState()
      if (own.self_mute !== exp.self_mute || own.self_deaf !== exp.self_deaf) reportState()
    },

    applyCapabilities() {
      const s = get()
      if (s.status === 'idle' || !s.channelId) return
      const channel = channelCapabilities(s.communityId, s.channelId)
      if (!channel) return
      const granted = livekitCapabilities(room)
      const caps: Capabilities = {
        canSpeak: channel.canSpeak && (granted?.canSpeak ?? true),
        canStream: channel.canStream && (granted?.canStream ?? true),
        canVideo: channel.canVideo && (granted?.canVideo ?? true),
      }
      if (caps.canSpeak === s.canSpeak && caps.canStream === s.canStream && caps.canVideo === s.canVideo) return
      set(caps)

      const r = room
      const stopped: string[] = []
      if (!caps.canStream && s.screenSharing) {
        void get()
          .stopScreenShare()
          .catch(() => {})
        stopped.push('Bildschirmübertragung')
      }
      // Auch eine vom Server nur stummgeschaltete Kamera ganz abbauen (Kameralicht aus).
      const cam = lk ? r?.localParticipant.getTrackPublication(lk.Track.Source.Camera) : undefined
      if (!caps.canVideo && r && cam?.track) {
        void r.localParticipant
          .unpublishTrack(cam.track, true)
          .catch(() => {})
          .finally(updateStreams)
        stopped.push('Kamera')
      }
      if (stopped.length > 0) set({ error: `${stopped.join(' und ')} beendet – dir fehlt dafür jetzt das Recht.` })
      get().refreshMic()
    },
  }
})

/** Video-Spur der Bildschirmübertragung eines Teilnehmers (auch die eigene). */
export function screenTrack(userId: string): Track | undefined {
  const r = room
  if (!r) return undefined
  const p = userId === r.localParticipant.identity ? r.localParticipant : r.remoteParticipants.get(userId)
  return p?.getTrackPublication(lk!.Track.Source.ScreenShare)?.track
}

/**
 * Kamera eines Teilnehmers für eine sichtbare Kachel anfordern (abonniert sie, falls fremd).
 * Gibt die Freigabe zurück – beim Unmount aufrufen, damit nicht mehr Sichtbares abbestellt wird.
 */
export function requestCamera(userId: string): () => void {
  cameraWants.set(userId, (cameraWants.get(userId) ?? 0) + 1)
  resync()
  let released = false
  return () => {
    if (released) return
    released = true
    const n = (cameraWants.get(userId) ?? 1) - 1
    if (n > 0) cameraWants.set(userId, n)
    else cameraWants.delete(userId)
    resync()
  }
}

/** Kamera-Spur eines Teilnehmers (auch die eigene); fremde nur, solange eine Kachel sie anfordert. */
export function cameraTrack(userId: string): Track | undefined {
  const r = room
  if (!r) return undefined
  const p = userId === r.localParticipant.identity ? r.localParticipant : r.remoteParticipants.get(userId)
  const pub = p?.getTrackPublication(lk!.Track.Source.Camera)
  return pub && !pub.isMuted ? pub.track : undefined
}

/** Während die PTT-Taste neu belegt wird, keinen PTT auslösen. */
export function setCapturingKey(active: boolean) {
  capturingKey = active
}

// ---------------------------------------------------------------------------
// Push-to-Talk – im Browser nur, solange das Fenster den Fokus hat.
// ---------------------------------------------------------------------------

function isTyping(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null
  const editable = !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)
  // Druckbare Tasten beim Tippen nicht abfangen, Sondertasten (F-Tasten, Strg …) schon.
  return editable && e.key.length === 1
}

window.addEventListener('keydown', (e) => {
  const v = useVoice.getState()
  if (capturingKey || v.status === 'idle' || v.settings.inputMode !== 'ptt') return
  if (e.code !== v.settings.pttKey || e.repeat || isTyping(e)) return
  useVoice.getState().setPttPressed(true)
})
window.addEventListener('keyup', (e) => {
  const v = useVoice.getState()
  if (v.settings.inputMode === 'ptt' && e.code === v.settings.pttKey) v.setPttPressed(false)
})
window.addEventListener('blur', () => useVoice.getState().setPttPressed(false))

/** Tastenkürzel im Call (wie bei Discord): Strg+Umschalt+M/D, K für die Kamera. */
export const CALL_SHORTCUTS = { mute: 'M', deaf: 'D', camera: 'K' } as const
export const shortcutLabel = (key: string) => `Strg+Umschalt+${key}`

window.addEventListener('keydown', (e) => {
  const v = useVoice.getState()
  if (capturingKey || v.status !== 'connected' || !e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey || e.repeat) return
  const key = e.key.toUpperCase()
  if (key === CALL_SHORTCUTS.mute) {
    if (v.canSpeak) v.toggleMute()
  } else if (key === CALL_SHORTCUTS.deaf) v.toggleDeaf()
  else if (key === CALL_SHORTCUTS.camera) void v.toggleCamera()
  else return
  e.preventDefault()
})

// ---------------------------------------------------------------------------
// Abgleich mit dem App-Store
// ---------------------------------------------------------------------------

useStore.subscribe((state, prev) => {
  const v = useVoice.getState()
  if (v.status === 'idle') return

  // Abgemeldet → auflegen.
  if (!state.user) {
    void v.leave()
    return
  }

  // Kanal/Community weg oder Rechte geändert.
  if (state.details !== prev.details && v.communityId && v.channelId) {
    const detail = state.details[v.communityId]
    if (!detail?.channels.some((c) => c.id === v.channelId)) {
      void v.leave()
      return
    }
    v.applyCapabilities()
  }

  // DM weg (Gruppe verlassen, Konto des Gegenübers gelöscht …) → Anruf beenden.
  if (state.dms !== prev.dms && v.communityId === null && v.channelId && !state.dms[v.channelId]) {
    void v.leave()
    return
  }

  if (state.voiceStates !== prev.voiceStates) v.checkServerState()
})

useCalls.subscribe((state, prev) => {
  const v = useVoice.getState()
  if (v.status !== 'idle' && v.communityId === null && state.calls !== prev.calls) v.checkServerState()
})
