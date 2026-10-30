// Anrufe in DMs: Zustand aus Server-Events, eingehende Anrufe, Klingelton. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { VoiceState } from './api.ts'
import {
  applyCallUpdate,
  applyVoiceState,
  callsFromServer,
  forgetCall,
  incomingCalls,
  isCallActive,
  isCallingOut,
  joinNames,
  removeVoiceState,
  ringtoneFor,
} from './callLogic.ts'
import type { Calls } from './callLogic.ts'

const vs = (user_id: string, channel_id: string, community_id: string | null = null): VoiceState => ({
  community_id,
  channel_id,
  user_id,
  self_mute: false,
  self_deaf: false,
  streaming: false,
  video: false,
  joined_at: '2026-10-30T12:00:00Z',
})

test('Stand aus READY: leere Einträge fallen weg', () => {
  const calls = callsFromServer([
    { channel_id: 'dm1', voice_states: [vs('alex', 'dm1')], ringing: ['bob'], caller_id: 'alex' },
    { channel_id: 'dm2', voice_states: [], ringing: [], caller_id: null },
  ])
  assert.deepEqual(Object.keys(calls), ['dm1'])
  assert.deepEqual(calls.dm1.ringing, ['bob'])
  assert.equal(calls.dm1.callerId, 'alex')
  assert.deepEqual(callsFromServer(undefined), {})
})

test('CALL_UPDATE: Klingeln beginnt und endet, Anrufer nur solange es klingelt', () => {
  let calls: Calls = {}
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: ['bob'], caller_id: 'alex' })
  assert.deepEqual(calls.dm1, { voiceStates: [], ringing: ['bob'], callerId: 'alex' })
  // Ende ohne Teilnehmer: Eintrag verschwindet.
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: [], caller_id: null })
  assert.deepEqual(calls, {})
  // Ende mit Teilnehmern: Eintrag bleibt, Anrufer weg.
  calls = applyVoiceState({}, vs('alex', 'dm1'))
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: ['bob'], caller_id: 'alex' })
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: [], caller_id: 'alex' })
  assert.equal(calls.dm1.callerId, null)
  assert.equal(calls.dm1.voiceStates.length, 1)
})

test('Voice-States: ein Nutzer ist nur in einem Raum', () => {
  let calls = applyVoiceState({}, vs('alex', 'dm1'))
  calls = applyVoiceState(calls, vs('bob', 'dm1'))
  assert.deepEqual(
    calls.dm1.voiceStates.map((v) => v.user_id),
    ['alex', 'bob'],
  )
  // Update (Mute) ersetzt an gleicher Stelle.
  calls = applyVoiceState(calls, { ...vs('alex', 'dm1'), self_mute: true })
  assert.deepEqual(
    calls.dm1.voiceStates.map((v) => [v.user_id, v.self_mute]),
    [
      ['alex', true],
      ['bob', false],
    ],
  )
  // Wechsel in einen anderen DM-Anruf.
  calls = applyVoiceState(calls, vs('bob', 'dm2'))
  assert.deepEqual(
    calls.dm1.voiceStates.map((v) => v.user_id),
    ['alex'],
  )
  assert.deepEqual(
    calls.dm2.voiceStates.map((v) => v.user_id),
    ['bob'],
  )
  // Wechsel in einen Sprachkanal einer Community: aus DM-Anrufen raus, dort nichts eingetragen.
  calls = applyVoiceState(calls, vs('alex', 'voice', 'community'))
  assert.deepEqual(Object.keys(calls), ['dm2'])
  // Verlassen.
  calls = removeVoiceState(calls, { channel_id: 'dm2', user_id: 'bob' })
  assert.deepEqual(calls, {})
  // Unbekanntes Verlassen ändert nichts (gleiches Objekt).
  const same = removeVoiceState(calls, { channel_id: 'dm9', user_id: 'x' })
  assert.equal(same, calls)
})

test('Eingehende Anrufe: nur wo es bei einem klingelt, nicht im eigenen Anruf, nicht ausgeblendet', () => {
  let calls: Calls = {}
  calls = applyCallUpdate(calls, { channel_id: 'dm2', ringing: ['me', 'carl'], caller_id: 'bob' })
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: ['me'], caller_id: 'alex' })
  calls = applyCallUpdate(calls, { channel_id: 'dm3', ringing: ['carl'], caller_id: 'me' })
  assert.deepEqual(incomingCalls(calls, 'me'), [
    { channelId: 'dm1', callerId: 'alex' },
    { channelId: 'dm2', callerId: 'bob' },
  ])
  assert.deepEqual(
    incomingCalls(calls, 'me', { currentChannelId: 'dm1', hidden: new Set(['dm2']) }),
    [],
  )
  assert.deepEqual(incomingCalls(calls, undefined), [])
  assert.deepEqual(incomingCalls(forgetCall(calls, 'dm1'), 'me'), [{ channelId: 'dm2', callerId: 'bob' }])
})

test('Ausgehendes Klingeln: man ist allein im Anruf und es klingelt bei anderen', () => {
  let calls = applyVoiceState({}, vs('me', 'dm1'))
  calls = applyCallUpdate(calls, { channel_id: 'dm1', ringing: ['bob'], caller_id: 'me' })
  assert.equal(isCallingOut(calls.dm1, 'me', true), true)
  // Noch nicht verbunden (Server kennt einen noch nicht): zählt auch.
  const pending = applyCallUpdate({}, { channel_id: 'dm1', ringing: ['bob'], caller_id: 'me' })
  assert.equal(isCallingOut(pending.dm1, 'me', true), true)
  // Nicht im Anruf, jemand anderes ist drin, niemand klingelt: nein.
  assert.equal(isCallingOut(calls.dm1, 'me', false), false)
  calls = applyVoiceState(calls, vs('carl', 'dm1'))
  assert.equal(isCallingOut(calls.dm1, 'me', true), false)
  assert.equal(isCallingOut(undefined, 'me', true), false)
  assert.equal(isCallActive(calls.dm1), true)
  assert.equal(isCallActive(pending.dm1), false)
})

test('Klingelton: eingehend vor ausgehend, „Bitte nicht stören“ nur für eingehend', () => {
  assert.equal(ringtoneFor({ incoming: 1, callingOut: true, dnd: false }), 'incomingCall')
  assert.equal(ringtoneFor({ incoming: 1, callingOut: false, dnd: true }), null)
  assert.equal(ringtoneFor({ incoming: 1, callingOut: true, dnd: true }), 'outgoingCall')
  assert.equal(ringtoneFor({ incoming: 0, callingOut: false, dnd: false }), null)
})

test('Namenslisten', () => {
  assert.equal(joinNames([]), '')
  assert.equal(joinNames(['Anna']), 'Anna')
  assert.equal(joinNames(['Anna', 'Ben']), 'Anna und Ben')
  assert.equal(joinNames(['Anna', 'Ben', 'Carla', 'Dora', 'Emil']), 'Anna, Ben, Carla und 2 weitere')
})
