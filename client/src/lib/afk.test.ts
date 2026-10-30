// AFK-Erkennung im Sprachkanal. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { afkTimeout, afkTimeoutLabel, nextLastActivity, shouldMoveToAfk } from './afk.ts'
import type { AfkInput } from './afk.ts'

const MIN = 60_000
const base: AfkInput = {
  now: 10 * MIN,
  lastActivity: 0,
  status: 'connected',
  communityId: 'c',
  channelId: 'voice',
  afkChannelId: 'afk',
  afkTimeoutSeconds: 300,
  screenSharing: false,
  cameraOn: false,
  canJoinAfk: true,
}

test('Nach der eingestellten Zeit ohne Aktivität: in den AFK-Kanal', () => {
  assert.equal(shouldMoveToAfk(base), true)
  assert.equal(shouldMoveToAfk({ ...base, lastActivity: 5 * MIN }), true)
  assert.equal(shouldMoveToAfk({ ...base, lastActivity: 5 * MIN + 1 }), false)
  // Längere Zeit eingestellt.
  assert.equal(shouldMoveToAfk({ ...base, afkTimeoutSeconds: 900 }), false)
  assert.equal(shouldMoveToAfk({ ...base, afkTimeoutSeconds: 60, lastActivity: 9 * MIN }), true)
})

test('Kein Wechsel ohne AFK-Kanal, im AFK-Kanal, ohne Recht, in DMs oder ohne Verbindung', () => {
  assert.equal(shouldMoveToAfk({ ...base, afkChannelId: null }), false)
  assert.equal(shouldMoveToAfk({ ...base, afkChannelId: undefined }), false)
  assert.equal(shouldMoveToAfk({ ...base, channelId: 'afk' }), false)
  assert.equal(shouldMoveToAfk({ ...base, canJoinAfk: false }), false)
  assert.equal(shouldMoveToAfk({ ...base, communityId: null }), false)
  assert.equal(shouldMoveToAfk({ ...base, status: 'connecting' }), false)
  assert.equal(shouldMoveToAfk({ ...base, status: 'idle', channelId: null }), false)
})

test('Nie beim Streamen oder mit Kamera', () => {
  assert.equal(shouldMoveToAfk({ ...base, screenSharing: true }), false)
  assert.equal(shouldMoveToAfk({ ...base, cameraOn: true }), false)
})

test('Sprechen und Push-to-Talk zählen als Aktivität', () => {
  const voice = { speaking: ['other'], selfId: 'me', pttPressed: false }
  assert.equal(nextLastActivity(100, 500, voice), 100)
  assert.equal(nextLastActivity(100, 500, { ...voice, speaking: ['other', 'me'] }), 500)
  assert.equal(nextLastActivity(100, 500, { ...voice, pttPressed: true }), 500)
  // Ohne eigene ID zählt fremdes Sprechen nie.
  assert.equal(nextLastActivity(100, 500, { ...voice, speaking: ['me'], selfId: undefined }), 100)
})

test('Ungültige Zeiten fallen auf 5 Minuten zurück', () => {
  assert.equal(afkTimeout(undefined), 300)
  assert.equal(afkTimeout(120), 300)
  assert.equal(afkTimeout(3600), 3600)
  assert.equal(afkTimeoutLabel(60), '1 Minute')
  assert.equal(afkTimeoutLabel(3600), '1 Stunde')
  assert.equal(shouldMoveToAfk({ ...base, afkTimeoutSeconds: 7, lastActivity: 9 * MIN }), false)
})
