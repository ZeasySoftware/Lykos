// Ton-Einstellungen: Bitrate, Lautstärken, Pegel, Sprachaktivierungs-Gate. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  BITRATE_DEFAULT,
  BITRATE_STEPS,
  DEFAULT_CHANNEL_AUDIO,
  GATE_HOLD_MS,
  GATE_INITIAL,
  LEVEL_FLOOR_DB,
  SOUND_OUTPUT_FOLLOWS_VOICE,
  audioPublishOptions,
  bitrateLabel,
  clampBitrate,
  clampPttDelay,
  clampVolume,
  dbToFraction,
  fractionToDb,
  playbackVolume,
  rmsToDb,
  soundSinkId,
  stepGate,
} from './audioLogic.ts'
import { defaultVoiceSettings, normalizeVoiceSettings } from './voiceSettings.ts'

test('Bitrate: kbit/s → bit/s, auf 32–256 begrenzt', () => {
  assert.equal(clampBitrate(16), 32)
  assert.equal(clampBitrate(300), 256)
  assert.equal(clampBitrate(95.6), 96)
  assert.equal(clampBitrate(Number.NaN), BITRATE_DEFAULT)
  for (const step of BITRATE_STEPS) assert.equal(clampBitrate(step), step)
  assert.deepEqual(audioPublishOptions({ bitrate: 128, stereo: true, dtx: false, red: false }), {
    audioPreset: { maxBitrate: 128_000 },
    dtx: false,
    red: false,
    forceStereo: true,
  })
  assert.equal(audioPublishOptions({ bitrate: 999 }).audioPreset.maxBitrate, 256_000)
  assert.equal(bitrateLabel(64), '64 kbit/s')
})

test('Bitrate: ohne Server-Angabe gelten die Standardwerte (64 kbit/s, Mono, DTX/RED an)', () => {
  const opts = audioPublishOptions(undefined)
  assert.deepEqual(opts, {
    audioPreset: { maxBitrate: DEFAULT_CHANNEL_AUDIO.bitrate * 1000 },
    dtx: true,
    red: true,
    forceStereo: false,
  })
  assert.equal(opts.audioPreset.maxBitrate, 64_000)
})

test('Wiedergabelautstärke: Teilnehmer × Ausgabe, bis 200 %, taub/stumm = 0', () => {
  assert.equal(playbackVolume({ output: 1, deaf: false }), 1)
  assert.equal(playbackVolume({ user: { volume: 1.5, muted: false }, output: 1, deaf: false }), 1.5)
  assert.equal(playbackVolume({ user: { volume: 3, muted: false }, output: 1, deaf: false }), 2)
  assert.equal(playbackVolume({ user: { volume: 2, muted: false }, output: 0.5, deaf: false }), 1)
  assert.equal(playbackVolume({ user: { volume: 1, muted: true }, output: 1, deaf: false }), 0)
  assert.equal(playbackVolume({ user: { volume: 1, muted: false }, output: 1, deaf: true }), 0)
  assert.equal(clampVolume(-1), 0)
  assert.equal(clampVolume(Number.NaN), 1)
})

test('Pegel: RMS → dBFS und Anteil für die Anzeige', () => {
  assert.equal(rmsToDb(1), 0)
  assert.equal(Math.round(rmsToDb(0.1)), -20)
  assert.equal(rmsToDb(0), LEVEL_FLOOR_DB)
  assert.equal(rmsToDb(1e-9), LEVEL_FLOOR_DB)
  assert.equal(dbToFraction(LEVEL_FLOOR_DB), 0)
  assert.equal(dbToFraction(0), 1)
  assert.equal(dbToFraction(-50), 0.5)
  assert.equal(dbToFraction(10), 1)
  for (const db of [-100, -73, -50, -12, 0]) assert.equal(fractionToDb(dbToFraction(db)), db)
})

test('Gate: öffnet über dem Schwellwert, hält nach, schließt danach', () => {
  let g = stepGate(GATE_INITIAL, -70, -50, 0)
  assert.equal(g.open, false)
  g = stepGate(g, -30, -50, 10)
  assert.equal(g.open, true)
  g = stepGate(g, -70, -50, 10 + GATE_HOLD_MS)
  assert.equal(g.open, true, 'Haltezeit')
  g = stepGate(g, -70, -50, 11 + GATE_HOLD_MS)
  assert.equal(g.open, false)
  // Genau auf dem Schwellwert zählt als laut.
  assert.equal(stepGate(GATE_INITIAL, -50, -50, 0).open, true)
})

test('Gate: Schwellwert ganz unten = immer offen', () => {
  assert.equal(stepGate(GATE_INITIAL, LEVEL_FLOOR_DB, LEVEL_FLOOR_DB, 0).open, true)
  assert.equal(stepGate(GATE_INITIAL, -100, -120, 5000).open, true)
})

test('Push-to-Talk-Verzögerung und Gerät für Hinweistöne', () => {
  assert.equal(clampPttDelay(-5), 0)
  assert.equal(clampPttDelay(5000), 1000)
  assert.equal(clampPttDelay(149.6), 150)
  assert.equal(soundSinkId({ soundOutputDeviceId: SOUND_OUTPUT_FOLLOWS_VOICE, outputDeviceId: 'kopfhörer' }), 'kopfhörer')
  assert.equal(soundSinkId({ soundOutputDeviceId: '', outputDeviceId: 'kopfhörer' }), '')
  assert.equal(soundSinkId({ soundOutputDeviceId: 'boxen', outputDeviceId: 'kopfhörer' }), 'boxen')
})

test('Gespeicherte Voice-Einstellungen: neue Felder ergänzt, Werte begrenzt', () => {
  const old = normalizeVoiceSettings({ inputMode: 'ptt', pttKey: 'F13' })
  assert.equal(old.inputMode, 'ptt')
  assert.equal(old.pttKey, 'F13')
  assert.equal(old.inputVolume, 1)
  assert.equal(old.outputVolume, 1)
  assert.equal(old.vadThreshold, defaultVoiceSettings.vadThreshold)
  assert.equal(old.soundOutputDeviceId, SOUND_OUTPUT_FOLLOWS_VOICE)
  const wild = normalizeVoiceSettings({ inputVolume: 9, outputVolume: -1, vadThreshold: -500, pttReleaseDelay: 99999 })
  assert.equal(wild.inputVolume, 2)
  assert.equal(wild.outputVolume, 0)
  assert.equal(wild.vadThreshold, LEVEL_FLOOR_DB)
  assert.equal(wild.pttReleaseDelay, 1000)
  assert.deepEqual(normalizeVoiceSettings(null), defaultVoiceSettings)
})
