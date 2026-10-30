// Beispiele für den Markdown-Parser. Ausführen mit `npm test` (Node mit Type-Stripping).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseBlocks, parseInline, plainText } from './markdownParser.ts'

const t = (text: string) => ({ type: 'text', text })

test('Hervorhebungen', () => {
  assert.deepEqual(parseInline('**fett** und *kursiv*'), [
    { type: 'bold', children: [t('fett')] },
    t(' und '),
    { type: 'italic', children: [t('kursiv')] },
  ])
  assert.deepEqual(parseInline('~~weg~~ ||geheim||'), [
    { type: 'strike', children: [t('weg')] },
    t(' '),
    { type: 'spoiler', children: [t('geheim')] },
  ])
  assert.deepEqual(parseInline('**fett *und kursiv* !**'), [
    { type: 'bold', children: [t('fett '), { type: 'italic', children: [t('und kursiv')] }, t(' !')] },
  ])
  assert.deepEqual(parseInline('_kursiv_ aber snake_case_name'), [
    { type: 'italic', children: [t('kursiv')] },
    t(' aber snake_case_name'),
  ])
})

test('Unvollständige Markierungen bleiben Text', () => {
  assert.deepEqual(parseInline('2 * 3 = 6 und **offen'), [t('2 * 3 = 6 und **offen')])
  assert.deepEqual(parseInline('a ** b'), [t('a ** b')])
})

test('Code schützt seinen Inhalt', () => {
  assert.deepEqual(parseInline('`**kein fett** @bob`'), [{ type: 'code', text: '**kein fett** @bob' }])
  assert.deepEqual(parseInline('\\*kein kursiv\\*'), [t('*kein kursiv*')])
})

test('Links nur für http/https, ohne Satzzeichen am Ende', () => {
  assert.deepEqual(parseInline('siehe https://example.org/a.'), [
    t('siehe '),
    { type: 'link', url: 'https://example.org/a' },
    t('.'),
  ])
  assert.deepEqual(parseInline('(https://de.wikipedia.org/wiki/Wolf_(Begriffsklärung))'), [
    t('('),
    { type: 'link', url: 'https://de.wikipedia.org/wiki/Wolf_(Begriffsklärung)' },
    t(')'),
  ])
  assert.deepEqual(parseInline('javascript:alert(1) ftp://x.org'), [t('javascript:alert(1) ftp://x.org')])
  assert.deepEqual(parseInline('https://'), [t('https://')])
})

test('Erwähnungen', () => {
  assert.deepEqual(parseInline('Hi @bob, mail@bob.de @everyone'), [
    t('Hi '),
    { type: 'mention', name: 'bob' },
    t(', mail@bob.de '),
    { type: 'everyone' },
  ])
})

test('Blöcke: Codeblock, Zitat, Absatz', () => {
  assert.deepEqual(parseBlocks('Vorher\n```rust\nfn main() {}\n```\nNachher'), [
    { type: 'paragraph', children: [t('Vorher')] },
    { type: 'codeblock', lang: 'rust', text: 'fn main() {}' },
    { type: 'paragraph', children: [t('Nachher')] },
  ])
  assert.deepEqual(parseBlocks('> Zitat\n> zweite Zeile\nAntwort'), [
    { type: 'quote', children: [{ type: 'paragraph', children: [t('Zitat\nzweite Zeile')] }] },
    { type: 'paragraph', children: [t('Antwort')] },
  ])
  assert.deepEqual(parseBlocks('```offen'), [{ type: 'paragraph', children: [t('```offen')] }])
  assert.deepEqual(parseBlocks('>kein Zitat'), [{ type: 'paragraph', children: [t('>kein Zitat')] }])
})

test('Klartext für Vorschauen', () => {
  assert.equal(plainText('**Hallo** ||geheim|| `x`'), 'Hallo ▒▒▒ x')
})
