// Platzhalter im Willkommenstext (gleiches Verhalten wie der Server). Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { renderWelcome } from './welcome.ts'

test('Platzhalter werden ersetzt, Unbekanntes bleibt', () => {
  assert.equal(
    renderWelcome('Hallo {user}, willkommen bei **{community}**! {user}', 'wolf', 'Rudel'),
    'Hallo @wolf, willkommen bei **Rudel**! @wolf',
  )
  assert.equal(renderWelcome('Ohne Platzhalter', 'wolf', 'Rudel'), 'Ohne Platzhalter')
  assert.equal(renderWelcome('{nix} {USER}', 'wolf', 'Rudel'), '{nix} {USER}')
  // Wie auf dem Server: `{community}` wird nach `{user}` eingesetzt.
  assert.equal(renderWelcome('{community}', 'wolf', 'A {user} B'), 'A {user} B')
})
