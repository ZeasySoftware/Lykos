/**
 * Anrufe in DMs – reine Logik ohne Store/DOM (Unit-Tests: `callLogic.test.ts`).
 *
 * Der Server meldet je DM-Kanal, wer im Anruf ist (`VOICE_STATE_*` mit `community_id: null`) und bei wem es
 * klingelt (`CALL_UPDATE`); mit READY/RESYNC kommt der komplette Stand (`dm_calls`). Hier wird daraus der
 * lokale Zustand gepflegt und abgeleitet, was die Oberfläche zeigt: eingehende Anrufe, ausgehendes Klingeln,
 * welcher Klingelton läuft.
 */
import type { CallUpdate, DmCall, VoiceState } from './api'

/** Anrufzustand eines DM-Kanals. */
export interface CallEntry {
  /** Wer im Anruf ist (Reihenfolge nach Beitritt). */
  voiceStates: VoiceState[]
  /** Bei wem es gerade klingelt. */
  ringing: string[]
  /** Wer angerufen hat, solange es klingelt. */
  callerId: string | null
}

/** channel_id → Anrufzustand; DMs ohne Anruf und ohne Klingeln fehlen. */
export type Calls = Record<string, CallEntry>

const EMPTY: CallEntry = { voiceStates: [], ringing: [], callerId: null }

function isEmpty(e: CallEntry): boolean {
  return e.voiceStates.length === 0 && e.ringing.length === 0
}

/** Eintrag setzen; leere Einträge fallen weg. Gibt bei unveränderter Lage dasselbe Objekt zurück. */
function put(calls: Calls, channelId: string, entry: CallEntry): Calls {
  if (isEmpty(entry)) {
    if (!(channelId in calls)) return calls
    const next = { ...calls }
    delete next[channelId]
    return next
  }
  return { ...calls, [channelId]: entry }
}

/** Kompletter Stand aus READY/RESYNC. */
export function callsFromServer(list: DmCall[] | undefined): Calls {
  let calls: Calls = {}
  for (const c of list ?? []) {
    calls = put(calls, c.channel_id, { voiceStates: c.voice_states, ringing: c.ringing, callerId: c.caller_id })
  }
  return calls
}

/** `CALL_UPDATE`: Klingelzustand eines Kanals ersetzen. */
export function applyCallUpdate(calls: Calls, u: CallUpdate): Calls {
  const prev = calls[u.channel_id] ?? EMPTY
  return put(calls, u.channel_id, { ...prev, ringing: u.ringing, callerId: u.ringing.length > 0 ? u.caller_id : null })
}

/**
 * `VOICE_STATE_UPDATE`: Ein Nutzer ist höchstens in einem Raum – aus allen DM-Anrufen austragen und, falls der
 * neue Raum ein DM-Anruf ist (`community_id: null`), dort eintragen bzw. ersetzen (Position bleibt).
 */
export function applyVoiceState(calls: Calls, vs: VoiceState): Calls {
  let next = calls
  for (const [channelId, entry] of Object.entries(calls)) {
    if (channelId === vs.channel_id && vs.community_id === null) continue
    if (entry.voiceStates.some((v) => v.user_id === vs.user_id)) {
      next = put(next, channelId, { ...entry, voiceStates: entry.voiceStates.filter((v) => v.user_id !== vs.user_id) })
    }
  }
  if (vs.community_id !== null) return next
  const entry = next[vs.channel_id] ?? EMPTY
  const exists = entry.voiceStates.some((v) => v.user_id === vs.user_id)
  const voiceStates = exists
    ? entry.voiceStates.map((v) => (v.user_id === vs.user_id ? vs : v))
    : [...entry.voiceStates, vs]
  return put(next, vs.channel_id, { ...entry, voiceStates })
}

/** `VOICE_STATE_DELETE` in einem DM-Anruf. */
export function removeVoiceState(calls: Calls, d: { channel_id: string; user_id: string }): Calls {
  const entry = calls[d.channel_id]
  if (!entry?.voiceStates.some((v) => v.user_id === d.user_id)) return calls
  return put(calls, d.channel_id, { ...entry, voiceStates: entry.voiceStates.filter((v) => v.user_id !== d.user_id) })
}

/** DM weg (Gruppe verlassen): Anrufzustand vergessen. */
export function forgetCall(calls: Calls, channelId: string): Calls {
  return put(calls, channelId, EMPTY)
}

export interface IncomingCall {
  channelId: string
  callerId: string | null
}

/**
 * Eingehende Anrufe: DMs, in denen es bei `me` klingelt – außer dem Anruf, in dem man schon ist, und außer
 * lokal ausgeblendeten (`hidden`, z. B. während das Ablehnen noch unterwegs ist). Neueste Kanäle zuerst ist
 * nicht nötig; die Reihenfolge folgt den Kanal-IDs, damit Karten nicht springen.
 */
export function incomingCalls(
  calls: Calls,
  me: string | undefined,
  opts: { currentChannelId?: string | null; hidden?: ReadonlySet<string> } = {},
): IncomingCall[] {
  if (!me) return []
  return Object.entries(calls)
    .filter(([id, c]) => c.ringing.includes(me) && id !== opts.currentChannelId && !opts.hidden?.has(id))
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([channelId, c]) => ({ channelId, callerId: c.callerId }))
}

/** Ruft man selbst gerade an (man ist drin, sonst niemand, und es klingelt bei anderen)? */
export function isCallingOut(entry: CallEntry | undefined, me: string | undefined, inThisCall: boolean): boolean {
  if (!entry || !me || !inThisCall || entry.ringing.length === 0) return false
  return !entry.voiceStates.some((v) => v.user_id !== me)
}

/** Läuft in dieser DM ein Anruf (jemand ist drin)? */
export function isCallActive(entry: CallEntry | undefined): boolean {
  return !!entry && entry.voiceStates.length > 0
}

export type RingtoneName = 'incomingCall' | 'outgoingCall'

/**
 * Welcher Klingelton läuft? Eingehend hat Vorrang; bei „Bitte nicht stören“ klingelt nichts Eingehendes.
 * Das Freizeichen beim eigenen Anrufen bleibt – das hat man selbst ausgelöst.
 */
export function ringtoneFor(s: { incoming: number; callingOut: boolean; dnd: boolean }): RingtoneName | null {
  if (s.incoming > 0 && !s.dnd) return 'incomingCall'
  if (s.callingOut) return 'outgoingCall'
  return null
}

/** Abstand zwischen zwei Klingeltönen (ms): Schleife, solange es klingelt. */
export const RINGTONE_INTERVAL_MS: Record<RingtoneName, number> = { incomingCall: 2600, outgoingCall: 3200 }

/** „Anna“, „Anna und Ben“, „Anna, Ben und Carla“, „Anna, Ben, Carla und 2 weitere“. */
export function joinNames(names: string[]): string {
  const listed = names.length > 3 ? [...names.slice(0, 3), `${names.length - 3} weitere`] : names
  return listed.length > 1 ? `${listed.slice(0, -1).join(', ')} und ${listed.at(-1)}` : (listed[0] ?? '')
}
