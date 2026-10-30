// Soundeffekte: Logik und Klangrezepte. Ausführen mit `npm test` (Node mit Type-Stripping).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  MESSAGE_PRIORITY,
  MESSAGE_SOUND_INTERVAL_MS,
  SOUND_INFO,
  SOUND_NAMES,
  createThrottle,
  defaultSoundSettings,
  membershipSounds,
  messageSoundFor,
  normalizeSoundSettings,
  voiceTransitionSounds,
  volumeToGain,
} from './soundLogic.ts'
import type { MessageSoundInput, VoiceSnapshot } from './soundLogic.ts'
import { RECIPES, noteToHz, recipeDuration } from './soundSynth.ts'

test('Standardwerte: PTT aus, alles andere an', () => {
  const s = defaultSoundSettings()
  assert.equal(s.enabled, true)
  assert.ok(s.volume > 0 && s.volume <= 1)
  assert.equal(s.events.pttDown, false)
  assert.equal(s.events.pttUp, false)
  for (const name of SOUND_NAMES) {
    if (name !== 'pttDown' && name !== 'pttUp') assert.equal(s.events[name], true, name)
  }
})

test('Einstellungen aus dem Speicher werden bereinigt', () => {
  assert.deepEqual(normalizeSoundSettings(null), defaultSoundSettings())
  assert.deepEqual(normalizeSoundSettings('kaputt'), defaultSoundSettings())
  const s = normalizeSoundSettings({ enabled: false, volume: 7, events: { mention: false, pttDown: true, foo: true } })
  assert.equal(s.enabled, false)
  assert.equal(s.volume, 1)
  assert.equal(s.events.mention, false)
  assert.equal(s.events.pttDown, true)
  assert.equal('foo' in s.events, false)
  assert.equal(normalizeSoundSettings({ volume: -1 }).volume, 0)
  assert.equal(normalizeSoundSettings({ volume: Number.NaN }).volume, defaultSoundSettings().volume)
  assert.equal(normalizeSoundSettings({ events: { message: 'ja' } }).events.message, true)
})

test('Lautstärke-Kurve', () => {
  assert.equal(volumeToGain(0), 0)
  assert.equal(volumeToGain(1), 1)
  assert.equal(volumeToGain(0.5), 0.25)
  assert.equal(volumeToGain(3), 1)
})

test('Drosselung: höchstens ein Nachrichtenton pro Intervall, Wichtigeres darf durch', () => {
  const t = createThrottle(MESSAGE_SOUND_INTERVAL_MS)
  const p = (n: 'message' | 'dm' | 'mention') => MESSAGE_PRIORITY[n] ?? 0
  assert.equal(t.allow(0, p('message')), true)
  assert.equal(t.allow(200, p('message')), false)
  assert.equal(t.allow(1400, p('message')), false)
  // Erwähnung ist wichtiger als die gerade gespielte Nachricht.
  assert.equal(t.allow(1450, p('mention')), true)
  assert.equal(t.allow(1500, p('dm')), false)
  assert.equal(t.allow(1600, p('mention')), false)
  assert.equal(t.allow(1450 + MESSAGE_SOUND_INTERVAL_MS, p('message')), true)
  t.reset()
  assert.equal(t.allow(1450 + MESSAGE_SOUND_INTERVAL_MS + 1, p('message')), true)
})

test('Nachrichtenton: wann und welcher', () => {
  const base: MessageSoundInput = {
    own: false,
    dm: false,
    mentioned: false,
    channelMuted: false,
    dnd: false,
    windowFocused: true,
    viewedChannelId: 'a',
    channelId: 'b',
    syncing: false,
  }
  assert.equal(messageSoundFor(base), 'message')
  assert.equal(messageSoundFor({ ...base, dm: true }), 'dm')
  assert.equal(messageSoundFor({ ...base, mentioned: true }), 'mention')
  assert.equal(messageSoundFor({ ...base, dm: true, mentioned: true }), 'mention')
  // Offener Kanal bei Fokus: still; ohne Fokus: Ton.
  assert.equal(messageSoundFor({ ...base, viewedChannelId: 'b' }), null)
  assert.equal(messageSoundFor({ ...base, viewedChannelId: 'b', windowFocused: false }), 'message')
  // Eigene Nachrichten, Synchronisation, Bitte nicht stören: still.
  assert.equal(messageSoundFor({ ...base, own: true }), null)
  assert.equal(messageSoundFor({ ...base, syncing: true }), null)
  assert.equal(messageSoundFor({ ...base, dnd: true, mentioned: true }), null)
  // Stummgeschaltet: nur Erwähnungen.
  assert.equal(messageSoundFor({ ...base, channelMuted: true }), null)
  assert.equal(messageSoundFor({ ...base, channelMuted: true, mentioned: true }), 'mention')
})

const idle: VoiceSnapshot = {
  status: 'idle',
  error: null,
  selfMute: false,
  selfDeaf: false,
  pttPressed: false,
  screenSharing: false,
  cameraOn: false,
}
const connected: VoiceSnapshot = { ...idle, status: 'connected' }

test('Voice: Beitritt, Verlassen, Verbindung', () => {
  const connecting: VoiceSnapshot = { ...idle, status: 'connecting' }
  const reconnecting: VoiceSnapshot = { ...idle, status: 'reconnecting' }
  assert.deepEqual(voiceTransitionSounds(idle, connecting), [])
  assert.deepEqual(voiceTransitionSounds(connecting, connected), ['voiceJoin'])
  assert.deepEqual(voiceTransitionSounds(connected, idle), ['voiceLeave'])
  assert.deepEqual(voiceTransitionSounds(connected, reconnecting), ['connectionLost'])
  assert.deepEqual(voiceTransitionSounds(reconnecting, connected), ['reconnected'])
  // Endgültig verloren nach Reconnect-Versuch: jetzt „verlassen“ (Verlust kam schon).
  assert.deepEqual(voiceTransitionSounds(reconnecting, { ...idle, error: 'weg' }), ['voiceLeave'])
  assert.deepEqual(voiceTransitionSounds(connected, { ...idle, error: 'weg' }), ['connectionLost'])
  // Abgebrochener Beitritt still, fehlgeschlagener meldet sich.
  assert.deepEqual(voiceTransitionSounds(connecting, idle), [])
  assert.deepEqual(voiceTransitionSounds(connecting, { ...idle, error: 'x' }), ['connectionLost'])
})

test('Voice: Mikrofon, Ton, PTT, Übertragung, Kamera', () => {
  assert.deepEqual(voiceTransitionSounds(connected, { ...connected, selfMute: true }), ['mute'])
  assert.deepEqual(voiceTransitionSounds({ ...connected, selfMute: true }, connected), ['unmute'])
  assert.deepEqual(voiceTransitionSounds(connected, { ...connected, selfDeaf: true }), ['deafen'])
  // Entmuten hebt Taubheit auf → nur ein Ton.
  assert.deepEqual(voiceTransitionSounds({ ...connected, selfDeaf: true, selfMute: true }, connected), ['undeafen'])
  assert.deepEqual(voiceTransitionSounds(connected, { ...connected, pttPressed: true }), ['pttDown'])
  assert.deepEqual(voiceTransitionSounds({ ...connected, pttPressed: true }, connected), ['pttUp'])
  assert.deepEqual(voiceTransitionSounds(connected, { ...connected, screenSharing: true }), ['screenShareStart'])
  assert.deepEqual(voiceTransitionSounds({ ...connected, cameraOn: true }, connected), ['cameraOff'])
  // Beim Auflegen nur der Verlassen-Ton, nicht „Kamera aus“/„Übertragung endet“/„PTT los“.
  assert.deepEqual(
    voiceTransitionSounds({ ...connected, cameraOn: true, screenSharing: true, pttPressed: true }, idle),
    ['voiceLeave'],
  )
  // Mute außerhalb eines Gesprächs gibt trotzdem Rückmeldung.
  assert.deepEqual(voiceTransitionSounds(idle, { ...idle, selfMute: true }), ['mute'])
})

test('Teilnehmer/Streams: ein Ton pro Richtung, ohne mich selbst', () => {
  assert.deepEqual(membershipSounds([], ['me'], 'me', 'peerJoin', 'peerLeave'), [])
  assert.deepEqual(membershipSounds(['a'], ['a', 'b', 'c'], 'me', 'peerJoin', 'peerLeave'), ['peerJoin'])
  assert.deepEqual(membershipSounds(['a', 'b'], [], 'me', 'peerJoin', 'peerLeave'), ['peerLeave'])
  assert.deepEqual(membershipSounds(['a'], ['b'], 'me', 'peerJoin', 'peerLeave'), ['peerJoin', 'peerLeave'])
  assert.deepEqual(membershipSounds(['a', 'me'], ['a'], 'me', 'peerStreamStart', 'peerStreamStop'), [])
})

test('Klangrezepte: jedes Ereignis hat einen kurzen Klang (60–400 ms)', () => {
  for (const name of SOUND_NAMES) {
    const r = RECIPES[name]
    assert.ok(r, name)
    assert.ok(SOUND_INFO[name].label.length > 0)
    const d = recipeDuration(r)
    assert.ok(d >= 0.06 && d <= 0.4, `${name}: ${d}s`)
    assert.ok(r.gain > 0 && r.gain <= 0.6, name)
    assert.ok(r.reverb >= 0 && r.reverb <= 0.35, name)
    for (const n of r.notes) assert.ok(noteToHz(n.note) > 100 && noteToHz(n.note) < 3000, `${name} ${n.note}`)
  }
  // PTT ist bewusst sehr leise.
  assert.ok(RECIPES.pttDown.gain < RECIPES.message.gain)
  // Erwähnung deutlicher als normale Nachricht.
  assert.ok(RECIPES.mention.gain > RECIPES.message.gain)
})

test('Tonnamen', () => {
  assert.equal(noteToHz('A4'), 440)
  assert.ok(Math.abs(noteToHz('A5') - 880) < 1e-9)
  assert.ok(Math.abs(noteToHz('C5') - 523.2511) < 1e-3)
  assert.equal(noteToHz('H5'), noteToHz('B5'))
  assert.throws(() => noteToHz('X9'))
})
