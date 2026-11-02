// Mitgliederliste nach Rollen gruppieren. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Role } from './api.ts'
import { groupMembers } from './memberGroups.ts'

const role = (id: string, position: number, is_default = false): Role => ({
  id,
  community_id: 'c',
  name: id === 'alle' ? '@alle' : id[0].toUpperCase() + id.slice(1),
  permissions: 0,
  position,
  color: null,
  is_default,
})
const member = (id: string, role_ids: string[] = [], bot = false) => ({ user: { id, bot }, role_ids })

const roles = [role('alle', 0, true), role('azubi', 3), role('ausbilder', 1), role('leer', 2)]

test('online nach höchster Rolle in Hierarchie-Reihenfolge, ohne Rolle unter „Online“', () => {
  const members = [
    member('anna', ['azubi']),
    member('ben', ['azubi', 'ausbilder']),
    member('cem'),
    member('dora', ['alle', 'azubi']),
  ]
  const groups = groupMembers(members, new Set(['anna', 'ben', 'cem', 'dora']), roles)
  assert.deepEqual(
    groups.map((g) => [g.title, g.items.map((m) => m.user.id)]),
    [
      ['Ausbilder', ['ben']],
      ['Azubi', ['anna', 'dora']],
      ['Online', ['cem']],
    ],
  )
  assert.equal(groups[0].key, 'ausbilder')
})

test('offline als eine Gruppe, innerhalb nach Rolle und sonst Eingangsreihenfolge; Bots separat', () => {
  const members = [
    member('anna', ['azubi']),
    member('bot', ['ausbilder'], true),
    member('cem'),
    member('dora', ['ausbilder']),
    member('emil', ['azubi']),
  ]
  const groups = groupMembers(members, new Set(['bot']), roles)
  assert.deepEqual(
    groups.map((g) => [g.title, g.items.map((m) => m.user.id)]),
    [
      ['Offline', ['dora', 'anna', 'emil', 'cem']],
      ['Bots', ['bot']],
    ],
  )
})

test('unbekannte Rollen-IDs zählen wie keine Rolle', () => {
  const groups = groupMembers([member('anna', ['weg'])], new Set(['anna']), roles)
  assert.deepEqual(groups.map((g) => g.key), ['online'])
})

test('leere Liste ergibt keine Gruppen', () => {
  assert.deepEqual(groupMembers([], new Set(), roles), [])
})
