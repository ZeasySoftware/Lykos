// Menü-Logik: Filtern, Tastaturnavigation, Positionierung. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  edgeIndex,
  moveIndex,
  placeAbove,
  placeAtPoint,
  placeBelow,
  placeSubmenu,
  typeaheadIndex,
  visibleItems,
} from './menu.ts'
import type { MenuItem } from './menu.ts'

const item = (id: string, extra: Partial<Extract<MenuItem, { type: 'item' }>> = {}): MenuItem => ({
  type: 'item',
  id,
  label: id,
  ...extra,
})
const sep: MenuItem = { type: 'separator' }
const ids = (items: MenuItem[]) => items.map((i) => ('id' in i ? i.id : i.type))

test('visibleItems entfernt ausgeblendete Einträge und räumt Trennlinien auf', () => {
  const items: MenuItem[] = [
    sep,
    item('a'),
    sep,
    sep,
    item('b', { hidden: true }),
    sep,
    item('c'),
    sep,
  ]
  assert.deepEqual(ids(visibleItems(items)), ['a', 'separator', 'c'])
})

test('visibleItems: Überschrift ohne Einträge und leere Untermenüs verschwinden', () => {
  const items: MenuItem[] = [
    { type: 'header', label: 'Rollen' },
    item('x', { hidden: true }),
    sep,
    item('sub', { submenu: [item('s1', { hidden: true })] }),
    item('ok', { submenu: [item('s2'), item('s3', { hidden: true })] }),
  ]
  const out = visibleItems(items)
  assert.deepEqual(ids(out), ['ok'])
  const ok = out[0]
  assert.ok(ok.type === 'item' && ok.submenu)
  assert.deepEqual(ids(ok.submenu), ['s2'])
})

test('visibleItems behält Überschrift mit Einträgen', () => {
  const out = visibleItems([{ type: 'header', label: 'H' }, item('a'), sep, item('b')])
  assert.deepEqual(ids(out), ['header', 'a', 'separator', 'b'])
})

test('moveIndex überspringt Trennlinien, deaktivierte Einträge und läuft um', () => {
  const items: MenuItem[] = [item('a'), sep, item('b', { disabled: true }), item('c'), { type: 'header', label: 'h' }]
  assert.equal(moveIndex(items, -1, 1), 0)
  assert.equal(moveIndex(items, -1, -1), 3)
  assert.equal(moveIndex(items, 0, 1), 3)
  assert.equal(moveIndex(items, 3, 1), 0)
  assert.equal(moveIndex(items, 0, -1), 3)
  assert.equal(edgeIndex(items, 'first'), 0)
  assert.equal(edgeIndex(items, 'last'), 3)
  assert.equal(moveIndex([], -1, 1), -1)
})

test('moveIndex: eigene Inhalte nur, wenn fokussierbar', () => {
  const items: MenuItem[] = [
    { type: 'custom', id: 'row', render: () => null, focusable: true },
    { type: 'custom', id: 'deko', render: () => null },
    item('a'),
  ]
  assert.equal(moveIndex(items, 0, 1), 2)
  assert.equal(moveIndex(items, 2, 1), 0)
})

test('typeaheadIndex springt zum nächsten passenden Eintrag', () => {
  const items: MenuItem[] = [item('Antworten'), item('Anpinnen'), item('Bearbeiten'), item('Löschen')]
  assert.equal(typeaheadIndex(items, -1, 'a'), 0)
  assert.equal(typeaheadIndex(items, 0, 'a'), 1)
  assert.equal(typeaheadIndex(items, 1, 'A'), 0)
  assert.equal(typeaheadIndex(items, 0, 'l'), 3)
  assert.equal(typeaheadIndex(items, 2, 'x'), 2)
})

const vp = { width: 1000, height: 800 }
const size = { width: 200, height: 300 }

test('placeAtPoint: rechts unterhalb des Zeigers, an Rändern gespiegelt', () => {
  assert.deepEqual(placeAtPoint(100, 100, size, vp), { left: 100, top: 100 })
  // Rechts kein Platz → links vom Zeiger.
  assert.deepEqual(placeAtPoint(900, 100, size, vp), { left: 700, top: 100 })
  // Unten kein Platz → oberhalb.
  assert.deepEqual(placeAtPoint(100, 700, size, vp), { left: 100, top: 400 })
  // Ecke unten rechts.
  assert.deepEqual(placeAtPoint(990, 790, size, vp), { left: 790, top: 490 })
})

test('placeAtPoint hält das Menü im Fenster, auch wenn es kaum passt', () => {
  // Spiegeln würde links/oben aus dem Fenster ragen → an den Rand geschoben.
  assert.deepEqual(placeAtPoint(150, 250, { width: 200, height: 700 }, { width: 300, height: 720 }), {
    left: 8,
    top: 8,
  })
  // Höher als das Fenster: oben bündig.
  assert.equal(placeAtPoint(10, 10, { width: 100, height: 900 }, vp).top, 8)
})

test('placeSubmenu: rechts neben dem Eintrag, sonst links', () => {
  const row = { left: 300, top: 200, right: 500, bottom: 232 }
  assert.deepEqual(placeSubmenu(row, size, vp), { left: 504, top: 194 })
  const nearRight = { left: 700, top: 200, right: 900, bottom: 232 }
  assert.deepEqual(placeSubmenu(nearRight, size, vp), { left: 496, top: 194 })
  // Unten zu wenig Platz → nach oben verschoben.
  const low = { left: 300, top: 700, right: 500, bottom: 732 }
  assert.equal(placeSubmenu(low, size, vp).top, 492)
})

test('placeBelow/placeAbove für Dropdowns', () => {
  const trigger = { left: 80, top: 10, right: 300, bottom: 52 }
  assert.deepEqual(placeBelow(trigger, size, vp), { left: 80, top: 58 })
  // Unten kein Platz → oberhalb.
  const low = { left: 80, top: 700, right: 300, bottom: 740 }
  assert.deepEqual(placeBelow(low, size, vp), { left: 80, top: 394 })
  const bar = { left: 8, top: 740, right: 240, bottom: 790 }
  assert.deepEqual(placeAbove(bar, size, vp), { left: 8, top: 434 })
  // Oben kein Platz → darunter.
  assert.equal(placeAbove(trigger, size, vp).top, 58)
})
