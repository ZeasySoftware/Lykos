// Logik der Call-Bühne. Ausführen mit `npm test` (Node mit Type-Stripping).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { MAX_GRID_TILES, buildTiles, featuredTile, gridLayout, pickVisible, tileKey } from './callLayout.ts'
import type { TileParticipant } from './callLayout.ts'

const p = (userId: string, camera = false, streaming = false): TileParticipant => ({ userId, camera, streaming })
const keys = (tiles: { key: string }[]) => tiles.map((t) => t.key)

test('Reihenfolge: Streams, dann Kameras, dann Avatare – sonst Beitrittsreihenfolge', () => {
  const tiles = buildTiles([p('a'), p('b', true), p('c', false, true), p('d', true, true), p('e')], ['d'])
  assert.deepEqual(keys(tiles), [
    'screen:d', // angesehen
    'screen:c',
    'user:b',
    'user:d',
    'user:a',
    'user:c',
    'user:e',
  ])
  assert.equal(tiles[0].watching, true)
  assert.equal(tiles[1].watching, false)
})

test('Eigener Stream zählt als angesehen (lokale Vorschau, kein Abo nötig)', () => {
  const tiles = buildTiles([p('me', false, true), p('x', false, true)], [], 'me')
  assert.deepEqual(keys(tiles).slice(0, 2), ['screen:me', 'screen:x'])
  assert.equal(tiles[0].watching, true)
})

test('pickVisible: alles sichtbar, solange es passt', () => {
  const tiles = buildTiles([p('a'), p('b')], [])
  assert.deepEqual(pickVisible(tiles, 12, new Set()), { visible: tiles, hidden: [] })
})

test('pickVisible: „+N“-Platz und Sprecher rücken nach vorn', () => {
  const tiles = buildTiles(
    Array.from({ length: 15 }, (_, i) => p(`u${i}`, i === 0)),
    [],
  )
  const none = pickVisible(tiles, MAX_GRID_TILES, new Set())
  assert.equal(none.visible.length, MAX_GRID_TILES - 1)
  assert.equal(none.hidden.length, 15 - (MAX_GRID_TILES - 1))

  // u13 spricht, ist aber eigentlich verborgen → verdrängt den hintersten stillen Avatar (u10).
  const { visible, hidden } = pickVisible(tiles, MAX_GRID_TILES, new Set(['u13']))
  assert.ok(visible.some((t) => t.userId === 'u13'))
  assert.ok(hidden.some((t) => t.userId === 'u10'))
  assert.equal(visible.length, MAX_GRID_TILES - 1)
  // Reihenfolge bleibt stabil (Kamera u0 vorn, u13 hinten).
  assert.equal(visible[0].userId, 'u0')
  assert.equal(visible.at(-1)!.userId, 'u13')
})

test('pickVisible: Kameras und Sprechende werden nicht verdrängt', () => {
  const tiles = buildTiles([p('a', true), p('b', true), p('c'), p('d')], [])
  // max 3 → 2 Plätze + „+N“; a, b haben Kamera; c, d verborgen. d spricht, aber nichts ist verdrängbar.
  const { visible } = pickVisible(tiles, 3, new Set(['d']))
  assert.deepEqual(keys(visible), ['user:a', 'user:b'])
})

test('gridLayout: sinnvolle Aufteilung für 1, 2, 4, 6, 9, 12 Kacheln (16:9, breite Fläche)', () => {
  const W = 1280
  const H = 720
  const cols = (n: number) => gridLayout(n, W, H, 8).cols
  assert.equal(cols(1), 1)
  assert.equal(cols(2), 2)
  assert.equal(cols(4), 2)
  assert.equal(cols(6), 3)
  assert.equal(cols(9), 3)
  assert.equal(cols(12), 4)
  const one = gridLayout(1, W, H, 8)
  assert.deepEqual([one.tileWidth, one.tileHeight], [1280, 720])
})

test('gridLayout: hohe, schmale Fläche stapelt untereinander', () => {
  const l = gridLayout(2, 400, 900, 8)
  assert.equal(l.cols, 1)
  assert.equal(l.rows, 2)
  assert.ok(l.tileWidth <= 400)
  assert.ok(l.tileHeight * 2 + 8 <= 900)
})

test('gridLayout: Kacheln passen immer in die Fläche', () => {
  for (const n of [1, 2, 3, 5, 7, 8, 10, 11, 12]) {
    for (const [w, h] of [
      [900, 500],
      [1600, 900],
      [600, 800],
    ]) {
      const l = gridLayout(n, w, h, 10)
      assert.ok(l.cols * l.tileWidth + (l.cols - 1) * 10 <= w, `Breite n=${n} ${w}x${h}`)
      assert.ok(l.rows * l.tileHeight + (l.rows - 1) * 10 <= h, `Höhe n=${n} ${w}x${h}`)
      assert.ok(l.cols * l.rows >= n)
    }
  }
  assert.deepEqual(gridLayout(0, 100, 100), { cols: 1, rows: 1, tileWidth: 0, tileHeight: 0 })
})

test('featuredTile: Fokus, dann letzter Sprecher mit Kamera, dann Stream, dann Kamera', () => {
  const tiles = buildTiles([p('me', true), p('a', true), p('b', true), p('c', false, true)], ['c'], 'me')
  assert.equal(featuredTile(tiles, tileKey('user', 'b'), 'a', 'me')?.key, 'user:b')
  assert.equal(featuredTile(tiles, null, 'a', 'me')?.key, 'user:a')
  assert.equal(featuredTile(tiles, null, null, 'me')?.key, 'screen:c')
  // Fokus auf Kachel ohne Bild zählt nicht.
  const noVideo = buildTiles([p('me'), p('a')], [], 'me')
  assert.equal(featuredTile(noVideo, tileKey('user', 'a'), 'a', 'me'), null)
  // Eigene Kamera allein wird nicht gezeigt.
  assert.equal(featuredTile(buildTiles([p('me', true)], [], 'me'), null, null, 'me'), null)
})
