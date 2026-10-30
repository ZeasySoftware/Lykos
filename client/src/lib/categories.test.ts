// Kanal-Kategorien in der Seitenleiste. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Category, Channel } from './api.ts'
import { knowsCategory, sidebarLayout, sortedCategories, upsertCategory } from './categories.ts'

function channel(id: string, kind: 'text' | 'voice', position: number, category_id: string | null = null): Channel {
  return {
    id,
    community_id: 'c',
    name: id,
    kind,
    topic: null,
    position,
    audio_bitrate: 64,
    audio_stereo: false,
    audio_dtx: true,
    audio_red: true,
    user_limit: null,
    category_id,
  }
}

const category = (id: string, position: number): Category => ({ id, community_id: 'c', name: id, position })

test('ohne Kategorien bleibt alles oben (bestehende Communities unverändert)', () => {
  const layout = sidebarLayout([channel('v', 'voice', 0), channel('b', 'text', 1), channel('a', 'text', 0)], undefined, false)
  assert.deepEqual(
    layout.uncategorized.text.map((c) => c.id),
    ['a', 'b'],
  )
  assert.deepEqual(
    layout.uncategorized.voice.map((c) => c.id),
    ['v'],
  )
  assert.deepEqual(layout.categories, [])
})

test('Kategorien nach Position, darin Text vor Sprache', () => {
  const layout = sidebarLayout(
    [
      channel('oben', 'text', 0),
      channel('g-voice', 'voice', 0, 'games'),
      channel('g-text2', 'text', 5, 'games'),
      channel('g-text1', 'text', 2, 'games'),
      channel('t-text', 'text', 1, 'talk'),
      channel('fremd', 'text', 3, 'unbekannt'),
    ],
    [category('games', 1), category('talk', 0)],
    false,
  )
  assert.deepEqual(
    layout.categories.map((s) => s.category.id),
    ['talk', 'games'],
  )
  const games = layout.categories[1]
  assert.deepEqual(
    games.text.map((c) => c.id),
    ['g-text1', 'g-text2'],
  )
  assert.deepEqual(
    games.voice.map((c) => c.id),
    ['g-voice'],
  )
  // Unbekannte Kategorie → oben bei den übrigen.
  assert.deepEqual(
    layout.uncategorized.text.map((c) => c.id),
    ['oben', 'fremd'],
  )
})

test('leere Kategorien nur für Kanal-Verwalter', () => {
  const cats = [category('leer', 0), category('voll', 1)]
  const channels = [channel('x', 'text', 0, 'voll')]
  assert.deepEqual(
    sidebarLayout(channels, cats, false).categories.map((s) => s.category.id),
    ['voll'],
  )
  assert.deepEqual(
    sidebarLayout(channels, cats, true).categories.map((s) => s.category.id),
    ['leer', 'voll'],
  )
})

test('Hilfen für Events und Auswahlfelder', () => {
  const list = [category('b', 1), category('a', 0)]
  assert.deepEqual(
    sortedCategories(list).map((c) => c.id),
    ['a', 'b'],
  )
  assert.deepEqual(sortedCategories(undefined), [])
  const renamed = { ...category('a', 0), name: 'Neu' }
  assert.equal(upsertCategory(list, renamed).find((c) => c.id === 'a')?.name, 'Neu')
  assert.equal(upsertCategory(list, category('c', 2)).length, 3)
  assert.equal(upsertCategory(undefined, category('c', 2)).length, 1)
  assert.ok(knowsCategory(list, null))
  assert.ok(knowsCategory(list, 'a'))
  assert.ok(!knowsCategory(list, 'z'))
  assert.ok(!knowsCategory(undefined, 'z'))
})
