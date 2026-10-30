// Custom-Emote-Tokens, Emoji-Erkennung und `:shortcode:`-Eingabe. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  completedShortcode,
  customReaction,
  emojiOnlyCount,
  emojiQuery,
  formatCustomToken,
  parseCustomReaction,
  parseCustomToken,
  reactionKey,
  replaceShortcodes,
  shortenCustomTokens,
} from './customEmoji.ts'
import { parseInline, plainText } from './markdownParser.ts'

const ID = '0192f3a0-1111-7000-8000-000000000001'

test('Tokens lesen und schreiben', () => {
  assert.deepEqual(parseCustomToken(`x<a:tanz_1:${ID}>`, 1), { animated: true, name: 'tanz_1', id: ID, length: 47 })
  assert.deepEqual(parseCustomToken(`<:wolf:${ID.toUpperCase()}>`, 0)?.id, ID)
  for (const bad of [`<:w:${ID}>`, `<:wolf:${ID}`, `<b:wolf:${ID}>`, '<:wolf:123>', `<:mit leer:${ID}>`]) {
    assert.equal(parseCustomToken(bad, 0), null, bad)
  }
  assert.equal(formatCustomToken({ name: 'wolf', id: ID, animated: false }), `<:wolf:${ID}>`)
  assert.equal(formatCustomToken({ name: 'tanz', id: ID, animated: true }), `<a:tanz:${ID}>`)
})

test('Reaktionsschlüssel', () => {
  assert.equal(customReaction({ name: 'wolf', id: ID }), `wolf:${ID}`)
  assert.deepEqual(parseCustomReaction(`wolf:${ID}`), { name: 'wolf', id: ID })
  assert.equal(parseCustomReaction('👍'), null)
  // Umbenannte Emotes bleiben dieselbe Reaktion.
  assert.equal(reactionKey(`alt:${ID}`), reactionKey(`neu:${ID}`))
  assert.equal(reactionKey('👍'), '👍')
})

test('Markdown: Emotes als eigener Knoten, auch mit Unterstrich im Namen', () => {
  assert.deepEqual(parseInline(`Hi <:wolf_heult:${ID}>!`), [
    { type: 'text', text: 'Hi ' },
    { type: 'emoji', name: 'wolf_heult', id: ID, animated: false },
    { type: 'text', text: '!' },
  ])
  assert.deepEqual(parseInline(`\`<:wolf:${ID}>\``), [{ type: 'code', text: `<:wolf:${ID}>` }])
  assert.equal(plainText(`**<a:tanz:${ID}>** los`), ':tanz: los')
})

test('Reine Emoji-Nachrichten erkennen (groß darstellen)', () => {
  assert.equal(emojiOnlyCount('🐺'), 1)
  assert.equal(emojiOnlyCount(` 👍🏽  🇩🇪 👨‍👩‍👧 1️⃣ <:wolf:${ID}> `), 5)
  assert.equal(emojiOnlyCount('🐺 hallo'), 0)
  assert.equal(emojiOnlyCount('1'), 0)
  assert.equal(emojiOnlyCount('©'), 0)
  assert.equal(emojiOnlyCount(''), 0)
  assert.equal(emojiOnlyCount('🐺'.repeat(27)), 27)
  assert.equal(emojiOnlyCount('🐺'.repeat(28)), 0)
})

test('Shortcodes beim Senden ersetzen, Tokens beim Bearbeiten kürzen', () => {
  const resolve = (code: string) => (code === 'wolf' ? `<:wolf:${ID}>` : code === 'joy' ? '😂' : null)
  assert.equal(
    replaceShortcodes(`:wolf: und :joy: aber nicht \`:wolf:\`, 12:30:45, :gibts: <:wolf:${ID}>`, resolve),
    `<:wolf:${ID}> und 😂 aber nicht \`:wolf:\`, 12:30:45, :gibts: <:wolf:${ID}>`,
  )
  assert.equal(replaceShortcodes('```\n:wolf:\n```:wolf:', resolve), `\`\`\`\n:wolf:\n\`\`\`<:wolf:${ID}>`)
  assert.equal(replaceShortcodes(':wolf::joy:', resolve), `<:wolf:${ID}>:joy:`)
  const short = (e: { id: string; name: string }) => (e.id === ID ? e.name : null)
  assert.equal(shortenCustomTokens(`a <a:wolf:${ID}> \`<:wolf:${ID}>\``, short), `a :wolf: \`<:wolf:${ID}>\``)
  const other = '0192f3a0-1111-7000-8000-000000000002'
  assert.equal(shortenCustomTokens(`<:x_y:${other}>`, short), `<:x_y:${other}>`)
})

test(':shortcode:-Eingabe', () => {
  assert.deepEqual(emojiQuery('Hallo :wol', 10), { start: 6, query: 'wol' })
  assert.deepEqual(emojiQuery(':+1', 3), { start: 0, query: '+1' })
  assert.equal(emojiQuery('Hallo :w', 8), null)
  assert.equal(emojiQuery('um 12:30', 8), null)
  assert.equal(emojiQuery('http://x', 6), null)
  assert.equal(emojiQuery(`<:wolf`, 6), null)
  assert.deepEqual(completedShortcode('Ja :thumbsup:', 13), { start: 3, code: 'thumbsup' })
  assert.equal(completedShortcode('12:30:', 6), null)
})
