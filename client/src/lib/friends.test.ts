// Freundesystem: Tabs, Suche, Aktionen, ausgeblendete Nachrichten. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PresenceStatus, Relationship, RelationshipType } from './api.ts'
import {
  blockedLabel,
  blockedRunStart,
  blockedRuns,
  friendActions,
  friendsInTab,
  incomingCount,
  matchesQuery,
  relationshipShowsProfile,
} from './friends.ts'

const rel = (id: string, type: RelationshipType, display_name: string | null = null): Relationship => ({
  id,
  type,
  user: { id, username: id, display_name, avatar_url: null },
  since: '2026-10-30T12:00:00Z',
})

const list = [
  rel('zoe', 'friend'),
  rel('anna', 'friend', 'Änne'),
  rel('bert', 'friend'),
  rel('carl', 'pending_out'),
  rel('dora', 'pending_in'),
  rel('emil', 'blocked'),
]
const online: Record<string, PresenceStatus> = { zoe: 'online', anna: 'dnd', bert: 'offline' }
const statusOf = (id: string): PresenceStatus => online[id] ?? 'offline'
const ids = (rs: Relationship[]) => rs.map((r) => r.id)

test('Tabs filtern und sortieren nach Anzeigename', () => {
  assert.deepEqual(ids(friendsInTab(list, 'online', '', statusOf)), ['anna', 'zoe'])
  assert.deepEqual(ids(friendsInTab(list, 'all', '', statusOf)), ['anna', 'bert', 'zoe'])
  // Eingehende Anfragen vor ausgehenden.
  assert.deepEqual(ids(friendsInTab(list, 'pending', '', statusOf)), ['dora', 'carl'])
  assert.deepEqual(ids(friendsInTab(list, 'blocked', '', statusOf)), ['emil'])
})

test('Suche nach Benutzer- und Anzeigename, ohne Groß-/Kleinschreibung', () => {
  assert.ok(matchesQuery(rel('anna', 'friend', 'Änne'), 'änn'))
  assert.ok(matchesQuery(rel('anna', 'friend', 'Änne'), 'ANN'))
  assert.ok(matchesQuery(rel('anna', 'friend'), '  '))
  assert.ok(!matchesQuery(rel('anna', 'friend'), 'bert'))
  assert.deepEqual(ids(friendsInTab(list, 'all', 'ZO', statusOf)), ['zoe'])
})

test('offene eingehende Anfragen zählen', () => {
  assert.equal(incomingCount(list), 1)
  assert.equal(incomingCount([]), 0)
})

test('Aktionen je Beziehung', () => {
  assert.deepEqual(friendActions(undefined, false), {
    add: true,
    accept: false,
    decline: false,
    cancel: false,
    remove: false,
    block: true,
    unblock: false,
  })
  const pendingIn = friendActions('pending_in', false)
  assert.ok(pendingIn.accept && pendingIn.decline && pendingIn.block && !pendingIn.add)
  assert.ok(friendActions('pending_out', false).cancel)
  const friend = friendActions('friend', false)
  assert.ok(friend.remove && friend.block && !friend.add)
  const blocked = friendActions('blocked', false)
  assert.ok(blocked.unblock && !blocked.block && !blocked.add)
  // Gegenüber sich selbst gibt es nichts.
  assert.ok(Object.values(friendActions(undefined, true)).every((v) => !v))
})

test('Profil über die Beziehung nur für Freunde und eingehende Anfragen', () => {
  assert.ok(relationshipShowsProfile('friend'))
  assert.ok(relationshipShowsProfile('pending_in'))
  assert.ok(!relationshipShowsProfile('pending_out'))
  assert.ok(!relationshipShowsProfile('blocked'))
})

test('Nachrichten blockierter Nutzer werden zu Läufen zusammengefasst', () => {
  const msgs = [
    { author: { id: 'a' } },
    { author: { id: 'x' } },
    { author: { id: 'x' } },
    { author: { id: 'b' } },
    { author: null },
    { author: { id: 'x' } },
  ]
  const runs = blockedRuns(msgs, (id) => id === 'x')
  assert.deepEqual([...runs], [
    [1, 2],
    [5, 1],
  ])
  assert.equal(blockedRunStart(runs, 2), 1)
  assert.equal(blockedRunStart(runs, 5), 5)
  assert.equal(blockedRunStart(runs, 3), null)
  assert.equal(blockedRuns(msgs, () => false).size, 0)
  assert.equal(blockedLabel(1), '1 Nachricht von einem blockierten Nutzer')
  assert.equal(blockedLabel(3), '3 Nachrichten von blockierten Nutzern')
})
