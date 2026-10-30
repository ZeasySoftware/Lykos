/**
 * Verdrahtung der Soundeffekte mit Voice-, App- und Updater-Store.
 *
 * Statt `playSound` in `voice.ts` zu verstreuen, beobachten wir hier die Zustandswechsel
 * (Beitritt, Verlassen, Verbindung, Mute/Deafen, PTT, Übertragungen, Kamera, andere Teilnehmer).
 * Wird in `main.tsx` einmal importiert. Nachrichten meldet `store.ts` direkt (`playMessageSound`).
 */
import { useStore } from '../store'
import { useUpdater } from './updater'
import { useVoice } from './voice'
import { useCalls } from './callStore'
import { soundSinkId } from './audioLogic'
import { membershipSounds, voiceTransitionSounds } from './soundLogic'
import type { SoundName, VoiceSnapshot } from './soundLogic'
import { isSoundSyncing, playSound, setSoundOutputDevice } from './sounds'

type VoiceState = ReturnType<typeof useVoice.getState>

function snapshot(v: VoiceState): VoiceSnapshot {
  return {
    status: v.status,
    error: v.error,
    selfMute: v.selfMute,
    selfDeaf: v.selfDeaf,
    pttPressed: v.pttPressed,
    screenSharing: v.screenSharing,
    cameraOn: v.cameraOn,
  }
}

function playAll(names: SoundName[]) {
  for (const name of names) playSound(name)
}

const inCall = (v: VoiceState) => v.status === 'connected' || v.status === 'reconnecting'

// ---------------------------------------------------------------------------
// Fremde Bildschirmübertragungen im eigenen Raum
// ---------------------------------------------------------------------------

/** Die erste Stream-Liste nach dem Beitritt ist der Ausgangsstand (kein Ton für laufende Streams). */
let streamsSeeded = false

function onStreams(v: VoiceState, prev: VoiceState) {
  if (!inCall(v)) {
    streamsSeeded = false
    return
  }
  if (v.streams === prev.streams) return
  if (!streamsSeeded) {
    streamsSeeded = true
    return
  }
  const self = useStore.getState().user?.id
  playAll(membershipSounds(prev.streams, v.streams, self, 'peerStreamStart', 'peerStreamStop'))
}

// ---------------------------------------------------------------------------
// Andere betreten/verlassen den eigenen Sprachkanal (Voice-States vom Server)
// ---------------------------------------------------------------------------

let peerChannel: string | null = null
let peers: string[] = []

function updatePeers() {
  const v = useVoice.getState()
  const app = useStore.getState()
  if (!inCall(v) || !v.channelId) {
    peerChannel = null
    peers = []
    return
  }
  // DM-Anrufe (`communityId: null`) führt der Anruf-Store.
  const states = v.communityId ? app.voiceStates[v.communityId] : useCalls.getState().calls[v.channelId]?.voiceStates
  const current = (states ?? []).filter((s) => s.channel_id === v.channelId).map((s) => s.user_id)
  if (peerChannel === v.channelId && !isSoundSyncing()) {
    playAll(membershipSounds(peers, current, app.user?.id, 'peerJoin', 'peerLeave'))
  }
  peerChannel = v.channelId
  peers = current
}

// ---------------------------------------------------------------------------
// Abonnements
// ---------------------------------------------------------------------------

useVoice.subscribe((v, prev) => {
  playAll(voiceTransitionSounds(snapshot(prev), snapshot(v)))
  onStreams(v, prev)
  if (v.status !== prev.status || v.channelId !== prev.channelId) updatePeers()
  if (soundSinkId(v.settings) !== soundSinkId(prev.settings)) setSoundOutputDevice(soundSinkId(v.settings))
})
// Hinweistöne: eigenes Gerät oder wie die Sprachausgabe (Einstellung „Benachrichtigungstöne“).
setSoundOutputDevice(soundSinkId(useVoice.getState().settings))

useStore.subscribe((s, prev) => {
  if (s.voiceStates !== prev.voiceStates) updatePeers()
})
useCalls.subscribe((s, prev) => {
  if (s.calls !== prev.calls) updatePeers()
})

useUpdater.subscribe((u, prev) => {
  if (u.status === 'ready' && (prev.status !== 'ready' || u.version !== prev.version)) playSound('updateReady')
})
