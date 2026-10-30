/**
 * Zustand der Anrufe in DMs (wer ist drin, bei wem klingelt es). Bewusst getrennt vom App-Store wie
 * `friendsStore.ts`; Gateway-Events reicht `store.ts` hierher durch (READY/RESYNC, `CALL_UPDATE`,
 * `VOICE_STATE_*` mit `community_id: null`). Logik in `callLogic.ts`, Aktionen in `callActions.ts`.
 */
import { create } from 'zustand'
import type { CallUpdate, DmCall, VoiceState } from './api'
import {
  applyCallUpdate,
  applyVoiceState,
  callsFromServer,
  forgetCall,
  removeVoiceState,
} from './callLogic'
import type { CallEntry, Calls } from './callLogic'

interface CallsState {
  calls: Calls
  /** Lokal ausgeblendete eingehende Anrufe (Ablehnen unterwegs), bis der Server das Klingeln beendet. */
  hidden: ReadonlySet<string>

  setAll: (list: DmCall[] | undefined) => void
  applyUpdate: (u: CallUpdate) => void
  applyVoiceState: (vs: VoiceState) => void
  removeVoiceState: (d: { channel_id: string; user_id: string }) => void
  forget: (channelId: string) => void
  hide: (channelId: string) => void
  reset: () => void
}

const NONE: ReadonlySet<string> = new Set()

/** Ausgeblendete Kanäle, in denen es nicht mehr bei einem klingelt, wieder freigeben. */
function pruneHidden(hidden: ReadonlySet<string>, calls: Calls, me: string | undefined): ReadonlySet<string> {
  if (hidden.size === 0) return hidden
  const next = new Set([...hidden].filter((id) => !!me && calls[id]?.ringing.includes(me)))
  return next.size === hidden.size ? hidden : next
}

/** Eigene Nutzer-ID – wird von `store.ts` gesetzt (vermeidet einen Import-Zyklus). */
let meId: string | undefined
export function setCallUser(id: string | undefined) {
  meId = id
}

export const useCalls = create<CallsState>()((set) => {
  function update(fn: (calls: Calls) => Calls) {
    set((s) => {
      const calls = fn(s.calls)
      if (calls === s.calls) return {}
      return { calls, hidden: pruneHidden(s.hidden, calls, meId) }
    })
  }
  return {
    calls: {},
    hidden: NONE,
    setAll: (list) => update(() => callsFromServer(list)),
    applyUpdate: (u) => update((c) => applyCallUpdate(c, u)),
    applyVoiceState: (vs) => update((c) => applyVoiceState(c, vs)),
    removeVoiceState: (d) => update((c) => removeVoiceState(c, d)),
    forget: (channelId) => update((c) => forgetCall(c, channelId)),
    hide: (channelId) => set((s) => ({ hidden: new Set([...s.hidden, channelId]) })),
    reset: () => set({ calls: {}, hidden: NONE }),
  }
})

const EMPTY_STATES: VoiceState[] = []

/** Anrufzustand eines DM-Kanals (oder `undefined`). */
export function useCall(channelId: string | null | undefined): CallEntry | undefined {
  return useCalls((s) => (channelId ? s.calls[channelId] : undefined))
}

/** Teilnehmer des Anrufs eines DM-Kanals. */
export function useCallStates(channelId: string | null | undefined): VoiceState[] {
  return useCalls((s) => (channelId ? (s.calls[channelId]?.voiceStates ?? EMPTY_STATES) : EMPTY_STATES))
}
