// Ringpuffer und Schwärzung der technischen Infos (Paket K). Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  MAX_ERRORS,
  RingBuffer,
  clearErrors,
  coarseOs,
  collectTechInfo,
  describe,
  installDiagnostics,
  recentErrors,
  recordError,
  redact,
  viewFromPath,
} from './diagnostics.ts'

test('Ringpuffer verdrängt die ältesten Einträge', () => {
  const buf = new RingBuffer<number>(3)
  for (let i = 1; i <= 5; i++) buf.push(i)
  assert.deepEqual(buf.toArray(), [3, 4, 5])
  assert.equal(buf.size, 3)
  buf.clear()
  assert.deepEqual(buf.toArray(), [])
})

test('Schwärzung: IDs, Tokens, E-Mail, Query-Strings', () => {
  assert.equal(
    redact('GET /api/channels/0192f3a4-5b6c-7d8e-9f01-23456789abcd/messages 404'),
    'GET /api/channels/<id>/messages 404',
  )
  assert.equal(redact('Authorization: Bearer abc.def-123'), 'Authorization: Bearer <redacted>')
  assert.equal(redact('token=Zx8k2Lq9Vb7Nm3Pw5Rt1Yu6Io4As0Df'), 'token=<redacted>')
  assert.equal(redact('Sitzung Zx8k2Lq9Vb7Nm3Pw5Rt1Yu6Io4As0Df abgelaufen'), 'Sitzung <redacted> abgelaufen')
  assert.equal(redact('Mail an alex@example.org fehlgeschlagen'), 'Mail an <email> fehlgeschlagen')
  assert.equal(
    redact('fetch https://lykos.zeasy.dev/api/attachments/x?sig=geheim failed'),
    'fetch https://lykos.zeasy.dev/api/attachments/x?… failed',
  )
  // Normale Fehlertexte bleiben lesbar.
  assert.equal(
    redact("TypeError: Cannot read properties of undefined (reading 'id')"),
    "TypeError: Cannot read properties of undefined (reading 'id')",
  )
  assert.equal(redact('x'.repeat(500), 10), `${'x'.repeat(9)}…`)
  assert.equal(redact('a\u0000b\nc'), 'a b\nc')
})

test('Werte werden ohne Objektinhalte beschrieben', () => {
  assert.equal(describe(new TypeError('kaputt')), 'TypeError: kaputt')
  assert.equal(describe('Senden fehlgeschlagen'), 'Senden fehlgeschlagen')
  assert.equal(describe(42), '42')
  // Eine Nachricht als Objekt darf nicht im Puffer landen.
  assert.equal(describe({ id: 'm1', content: 'privater Text', author: { username: 'bob' } }), '[Objekt]')
  assert.equal(describe({ name: 'ApiError', message: 'API-Fehler 500: internal' }), 'ApiError: API-Fehler 500: internal')
  assert.equal(describe({ code: 'network' }), '[Code network]')
})

test('Betriebssystem grob, Ansicht ohne IDs', () => {
  assert.equal(coarseOs('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'), 'Windows')
  assert.equal(coarseOs('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)'), 'macOS')
  assert.equal(coarseOs('Mozilla/5.0 (X11; Linux x86_64)'), 'Linux')
  assert.equal(coarseOs('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)'), 'iOS')
  assert.equal(coarseOs('Mozilla/5.0 (Linux; Android 15)'), 'Android')
  assert.equal(coarseOs('curl/8'), null)
  assert.equal(viewFromPath('/invite/AbC123xy'), '/invite/<code>')
  assert.equal(viewFromPath('/channels/0192f3a4-5b6c-7d8e-9f01-23456789abcd'), '/channels/<id>')
  assert.equal(viewFromPath('/'), '/')
})

test('Erfassen über console.error, onerror und unhandledrejection', () => {
  clearErrors()
  const listeners: Record<string, (e: never) => void> = {}
  const target = {
    addEventListener: (type: string, cb: (e: never) => void) => {
      listeners[type] = cb
    },
  }
  const printed: unknown[][] = []
  const con = { error: (...args: unknown[]) => printed.push(args) } as unknown as Console
  installDiagnostics(target as never, con)

  con.error('Laden fehlgeschlagen', new Error('API-Fehler 0: network'), { content: 'geheime Nachricht' })
  listeners.error({ message: 'x', filename: 'https://lykos.zeasy.dev/assets/index-abc.js?v=1', lineno: 12, error: new RangeError('zu groß') } as never)
  listeners.unhandledrejection({ reason: new Error('Token Zx8k2Lq9Vb7Nm3Pw5Rt1Yu6Io4As0Df ungültig') } as never)

  // Originale Ausgabe bleibt erhalten.
  assert.equal(printed.length, 1)
  const errors = recentErrors().map((e) => e.message)
  assert.deepEqual(errors, [
    'Laden fehlgeschlagen Error: API-Fehler 0: network [Objekt]',
    'RangeError: zu groß (index-abc.js:12)',
    'Unbehandelte Ablehnung: Error: Token <redacted> ungültig',
  ])
  assert.ok(!JSON.stringify(recentErrors()).includes('geheime Nachricht'))

  // Höchstens MAX_ERRORS Einträge, die neuesten bleiben.
  for (let i = 0; i < MAX_ERRORS + 10; i++) recordError([`Fehler ${i}`])
  const kept = recentErrors()
  assert.equal(kept.length, MAX_ERRORS)
  assert.equal(kept.at(-1)?.message, `Fehler ${MAX_ERRORS + 9}`)
  clearErrors()
})

test('Technische Infos enthalten nur das Angekündigte', () => {
  const info = collectTechInfo(
    {
      appVersion: '0.3.0',
      desktop: true,
      userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)',
      width: 1280.4,
      height: 720,
      pathname: '/',
      area: 'community/textkanal',
    },
    [{ at: '2026-10-29T12:00:00.000Z', message: 'boom' }],
  )
  assert.deepEqual(info, {
    app_version: '0.3.0',
    platform: 'desktop',
    os: 'Windows',
    window: { width: 1280, height: 720 },
    view: 'community/textkanal',
    errors: [{ at: '2026-10-29T12:00:00.000Z', message: 'boom' }],
  })
  assert.deepEqual(Object.keys(info).sort(), ['app_version', 'errors', 'os', 'platform', 'view', 'window'])
})
