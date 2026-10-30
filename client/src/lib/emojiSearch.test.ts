// Suche und Shortcode-Vorschläge gegen die echten Emoji-Daten. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  applySkin,
  findShortcode,
  fold,
  prepareEmojis,
  pushRecent,
  searchCustom,
  searchEmojis,
  shortcodeSuggestions,
} from './emojiSearch.ts'
import type { RawEmojiRow } from './emojiSearch.ts'

const raw = JSON.parse(readFileSync(new URL('../data/emoji-data.json', import.meta.url), 'utf8')) as {
  emojis: RawEmojiRow[]
}
const list = prepareEmojis(raw.emojis)
const first = (q: string) => searchEmojis(list, q)[0]?.emoji

const custom = [
  { id: 'a', name: 'wolf_heult', animated: false, community_id: 'c1' },
  { id: 'b', name: 'thumbsup_wolf', animated: true, community_id: 'c2' },
  { id: 'c', name: 'Joy', animated: false, community_id: 'c1' },
]

test('Daten sind vollständig', () => {
  assert.ok(list.length > 1500)
  assert.ok(list.every((e) => e.emoji && e.shortcodes.length > 0 && e.name))
})

test('Suche: deutsch, englisch, Shortcode, ohne Umlaute', () => {
  assert.equal(first('thumbsup'), '👍')
  assert.equal(first('daumen hoch'), '👍')
  assert.equal(first('wolf'), '🐺')
  assert.equal(first('joy'), '😂')
  // Ganzes Wort vor Wortanfang im englischen Shortcode („bosnia_herzegovina“).
  assert.notEqual(first('herz'), '🇧🇦')
  assert.ok(searchEmojis(list, 'herz').slice(0, 10).some((e) => e.emoji === '❤️'))
  assert.ok(searchEmojis(list, 'lächeln').length > 3)
  assert.deepEqual(searchEmojis(list, 'lacheln').slice(0, 5), searchEmojis(list, 'lächeln').slice(0, 5))
  assert.ok(searchEmojis(list, 'pizza').some((e) => e.emoji === '🍕'))
  assert.ok(searchEmojis(list, 'deutschland').some((e) => e.emoji === '🇩🇪'))
  assert.deepEqual(searchEmojis(list, 'xyzquatsch'), [])
  assert.deepEqual(searchEmojis(list, '   '), [])
})

test('Hauttöne', () => {
  const thumbs = list.find((e) => e.emoji === '👍')!
  assert.equal(applySkin(thumbs, 0), '👍')
  assert.equal(applySkin(thumbs, 3), '👍🏽')
  const wolf = list.find((e) => e.emoji === '🐺')!
  assert.equal(applySkin(wolf, 3), '🐺')
  // Sequenzen mit Varianten-Selektor: Modifier ersetzt FE0F.
  const writing = list.find((e) => e.emoji === '✍️')!
  assert.equal(applySkin(writing, 1), '✍🏻')
})

test('Shortcode-Vorschläge mischen Custom-Emotes und Unicode', () => {
  const s = shortcodeSuggestions(list, custom, 'thumbs')
  assert.equal(s[0].kind === 'custom' && s[0].emoji.id, 'b')
  assert.ok(s.some((x) => x.kind === 'unicode' && x.emoji.emoji === '👍'))
  const joy = shortcodeSuggestions(list, custom, 'joy')
  // Exakte Treffer zuerst; Custom-Emotes vor Unicode bei gleichem Rang.
  assert.equal(joy[0].kind, 'custom')
  assert.equal(joy[1].kind === 'unicode' && joy[1].emoji.emoji, '😂')
  assert.deepEqual(shortcodeSuggestions(list, custom, 'w'), [])
  assert.ok(shortcodeSuggestions(null, custom, 'wolf').every((x) => x.kind === 'custom'))
  assert.equal(shortcodeSuggestions(list, custom, 'th', 3).length, 3)
})

test('Exakter Shortcode und Custom-Suche', () => {
  assert.equal(findShortcode(list, custom, 'wolf_heult')?.kind, 'custom')
  const u = findShortcode(list, custom, 'thumbsup')
  assert.equal(u?.kind === 'unicode' && u.emoji.emoji, '👍')
  assert.equal(findShortcode(list, custom, 'gibtsnicht'), null)
  assert.deepEqual(
    searchCustom(custom, 'wolf').map((e) => e.id),
    ['a', 'b'],
  )
})

test('Hilfen', () => {
  assert.equal(fold('Größe Ärger'), 'grosse arger')
  assert.deepEqual(pushRecent(['a', 'b', 'c'], 'b'), ['b', 'a', 'c'])
  assert.deepEqual(pushRecent(['a', 'b'], 'c', 2), ['c', 'a'])
})
