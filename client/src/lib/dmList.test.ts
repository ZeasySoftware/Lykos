// DM-Seitenleiste: Abschnitte, Sortierung, Personenauswahl. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { DmChannel } from './api.ts'
import { addableContacts, dmActivity, dmSections, joinNames, remainingSlots } from './dmList.ts'

const user = (id: string) => ({ id, username: id, display_name: null, avatar_url: null })

// UUIDv7-artige IDs: lexikografisch = zeitlich.
const dm = (id: string, opts: Partial<DmChannel> = {}): DmChannel => ({
  id,
  kind: 'dm',
  name: null,
  recipients: [user('me'), user(`u-${id}`)],
  last_message_id: null,
  last_read_message_id: null,
  mention_count: 0,
  pinned_at: null,
  ...opts,
})

const ids = (list: DmChannel[]) => list.map((d) => d.id)

test('Aktivität: letzte Nachricht, sonst Erstellung', () => {
  assert.equal(dmActivity({ id: '01', last_message_id: null }), '01')
  assert.equal(dmActivity({ id: '01', last_message_id: '05' }), '05')
})

test('ohne Pins: neueste Aktivität zuerst, leere 1:1-DMs ausgeblendet', () => {
  const { pinned, recent } = dmSections([
    dm('01', { last_message_id: '10' }),
    dm('02', { last_message_id: '30' }),
    dm('03'), // leer, nie geschrieben
    dm('04', { kind: 'group_dm' }), // leere Gruppe bleibt sichtbar
  ])
  assert.deepEqual(pinned, [])
  assert.deepEqual(ids(recent), ['02', '01', '04'])
})

test('leere 1:1-DM erscheint, wenn ausgewählt oder im Anruf', () => {
  const list = [dm('01'), dm('02'), dm('03')]
  assert.deepEqual(ids(dmSections(list, { selectedId: '01' }).recent), ['01'])
  assert.deepEqual(ids(dmSections(list, { callIds: new Set(['02']) }).recent), ['02'])
  assert.deepEqual(ids(dmSections(list, { callIds: { '03': {} } }).recent), ['03'])
})

test('angepinnte stehen getrennt oben, zuletzt angepinnte zuerst – auch leere 1:1-DMs', () => {
  const { pinned, recent } = dmSections([
    dm('01', { last_message_id: '90', pinned_at: '2026-10-30T10:00:00Z' }),
    dm('02', { pinned_at: '2026-10-30T12:00:00Z' }), // leer, aber angepinnt
    dm('03', { last_message_id: '95' }),
    dm('04', { kind: 'group_dm', pinned_at: '2026-10-30T11:00:00.500Z' }),
  ])
  assert.deepEqual(ids(pinned), ['02', '04', '01'])
  assert.deepEqual(ids(recent), ['03'])
})

test('neue Nachricht verschiebt angepinnte nicht', () => {
  const before = dmSections([
    dm('01', { last_message_id: '10', pinned_at: '2026-10-30T12:00:00Z' }),
    dm('02', { last_message_id: '20', pinned_at: '2026-10-30T10:00:00Z' }),
  ])
  const after = dmSections([
    dm('01', { last_message_id: '10', pinned_at: '2026-10-30T12:00:00Z' }),
    dm('02', { last_message_id: '99', pinned_at: '2026-10-30T10:00:00Z' }),
  ])
  assert.deepEqual(ids(before.pinned), ids(after.pinned))
})

test('gleicher Anpinn-Zeitpunkt: nach Aktivität', () => {
  const at = '2026-10-30T12:00:00Z'
  const { pinned } = dmSections([
    dm('01', { last_message_id: '10', pinned_at: at }),
    dm('02', { last_message_id: '20', pinned_at: at }),
  ])
  assert.deepEqual(ids(pinned), ['02', '01'])
})

test('Personenauswahl blendet enthaltene aus', () => {
  const contacts = [user('anna'), user('bert'), user('carl')]
  assert.deepEqual(
    addableContacts(contacts, [user('me'), user('bert')]).map((u) => u.id),
    ['anna', 'carl'],
  )
})

test('Namen aufzählen', () => {
  assert.equal(joinNames([]), '')
  assert.equal(joinNames(['Anna']), 'Anna')
  assert.equal(joinNames(['Anna', 'Bert']), 'Anna und Bert')
  assert.equal(joinNames(['Anna', 'Bert', 'Carl']), 'Anna, Bert und Carl')
})

test('freie Plätze bis 10', () => {
  assert.equal(remainingSlots(1), 9)
  assert.equal(remainingSlots(2), 8)
  assert.equal(remainingSlots(10), 0)
  assert.equal(remainingSlots(12), 0)
})
