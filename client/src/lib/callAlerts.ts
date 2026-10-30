/**
 * Klingeln hörbar und sichtbar machen: Klingelton (Schleife, solange es klingelt), Freizeichen beim eigenen
 * Anrufen und eine Desktop-Benachrichtigung, wenn das Fenster im Hintergrund ist. Bei „Bitte nicht stören“
 * kein Klingelton und keine Benachrichtigung. Beides lässt sich in den Sound-Einstellungen abschalten
 * (`incomingCall`, `outgoingCall`). Wird in `main.tsx` einmal importiert.
 */
import { RINGTONE_INTERVAL_MS, incomingCalls, isCallingOut, ringtoneFor } from './callLogic'
import type { RingtoneName } from './callLogic'
import { useCalls } from './callStore'
import { notify } from './notify'
import { playSound } from './sounds'
import { useVoice } from './voice'
import { displayName, dmTitle, useStore } from '../store'

/** Abstand der beiden Klingeltöne eines „ring-ring“ (ms). */
const DOUBLE_RING_MS = 420

let current: RingtoneName | null = null
let loop: number | undefined
let second: number | undefined

function ring(name: RingtoneName) {
  playSound(name)
  if (name === 'incomingCall') second = window.setTimeout(() => playSound(name), DOUBLE_RING_MS)
}

function setRingtone(name: RingtoneName | null) {
  if (name === current) return
  window.clearInterval(loop)
  window.clearTimeout(second)
  loop = undefined
  current = name
  if (!name) return
  ring(name)
  loop = window.setInterval(() => ring(name), RINGTONE_INTERVAL_MS[name])
}

/** Schon gemeldete eingehende Anrufe (Kanal-IDs) – je Klingeln nur eine Benachrichtigung. */
let notified = new Set<string>()

function update() {
  const app = useStore.getState()
  const voice = useVoice.getState()
  const { calls, hidden } = useCalls.getState()
  const me = app.user?.id
  const currentChannel = voice.status === 'idle' ? null : voice.channelId
  const incoming = incomingCalls(calls, me, { currentChannelId: currentChannel, hidden })
  const dnd = app.chosenStatus === 'dnd'
  const callingOut =
    !!currentChannel && voice.communityId === null && isCallingOut(calls[currentChannel], me, true)
  setRingtone(ringtoneFor({ incoming: incoming.length, callingOut, dnd }))

  const ids = new Set(incoming.map((c) => c.channelId))
  for (const c of incoming) {
    if (notified.has(c.channelId) || dnd || document.hasFocus()) continue
    const dm = app.dms[c.channelId]
    const caller = dm?.recipients.find((u) => u.id === c.callerId)
    const who = caller ? displayName(caller) : 'Jemand'
    const where = dm?.kind === 'group_dm' ? ` in „${dmTitle(dm, me)}“` : ''
    void notify('Eingehender Anruf', `${who} ruft dich${where} an.`)
  }
  notified = ids
}

useCalls.subscribe(update)
useVoice.subscribe((v, prev) => {
  if (v.status !== prev.status || v.channelId !== prev.channelId) update()
})
useStore.subscribe((s, prev) => {
  if (s.chosenStatus !== prev.chosenStatus || s.user !== prev.user || s.dms !== prev.dms) update()
})
