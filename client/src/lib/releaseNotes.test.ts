// Beispiele für Release-Notizen auf der Download-Seite. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseNoteInline, parseReleaseNotes } from './releaseNotes.ts'

const t = (text: string) => ({ type: 'text', text })

test('Überschriften, Listen und Absätze', () => {
  assert.deepEqual(parseReleaseNotes('## Neu\n- **Freunde:** Anfragen\n- Anrufe\n\nDanke!'), [
    { type: 'heading', level: 2, children: [t('Neu')] },
    {
      type: 'list',
      items: [[{ type: 'bold', children: [t('Freunde:')] }, t(' Anfragen')], [t('Anrufe')]],
    },
    { type: 'paragraph', children: [t('Danke!')] },
  ])
})

test('Absatzzeilen werden zusammengefügt, Windows-Zeilenenden egal', () => {
  assert.deepEqual(parseReleaseNotes('Zeile eins\r\nZeile zwei'), [
    { type: 'paragraph', children: [t('Zeile eins Zeile zwei')] },
  ])
})

test('Benannte Links nur mit http/https', () => {
  assert.deepEqual(parseNoteInline('Siehe [CHANGELOG](https://example.org/c.md).'), [
    t('Siehe '),
    { type: 'named-link', url: 'https://example.org/c.md', children: [t('CHANGELOG')] },
    t('.'),
  ])
  // javascript:-Links werden nicht zu Links.
  assert.deepEqual(parseNoteInline('[x](javascript:alert(1))'), [t('[x](javascript:alert(1))')])
})

test('Code und Fettdruck bleiben erhalten', () => {
  assert.deepEqual(parseNoteInline('`setup.exe` **jetzt**'), [
    { type: 'code', text: 'setup.exe' },
    t(' '),
    { type: 'bold', children: [t('jetzt')] },
  ])
})

test('Rechtstexte behalten Zeilenumbrüche im Absatz', () => {
  assert.deepEqual(parseReleaseNotes('Muster GmbH\nHauptstraße 1', { keepLineBreaks: true }), [
    { type: 'paragraph', children: [t('Muster GmbH'), { type: 'break' }, t('Hauptstraße 1')] },
  ])
  assert.deepEqual(parseReleaseNotes('Muster GmbH\nHauptstraße 1'), [
    { type: 'paragraph', children: [t('Muster GmbH Hauptstraße 1')] },
  ])
})
