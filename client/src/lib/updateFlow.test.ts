// Zustandslogik des Desktop-Updaters. Ausführen mit `npm test` (Node mit Type-Stripping).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  updateCheckView,
  indicatorView,
  initialSplash,
  isStalled,
  percentOf,
  restartBlocked,
  shouldDownload,
  splashReducer,
  splashText,
  withTimeout,
  type SplashEvent,
  type SplashState,
} from './updateFlow.ts'

const run = (...events: SplashEvent[]): SplashState => events.reduce(splashReducer, initialSplash)

test('Splash: kein Update → direkt in die App', () => {
  assert.equal(splashText(initialSplash), 'Suche nach Updates …')
  assert.deepEqual(run({ type: 'no-update' }), { phase: 'done' })
  assert.equal(splashText(run({ type: 'no-update' })), 'Lykos wird geladen …')
})

test('Splash: Update laden mit Fortschritt, dann installieren', () => {
  const loading = run(
    { type: 'found', version: '0.3.2' },
    { type: 'started', total: 1000 },
    { type: 'progress', bytes: 300 },
    { type: 'progress', bytes: 333 },
  )
  assert.deepEqual(loading, { phase: 'downloading', version: '0.3.2', downloaded: 633, total: 1000 })
  assert.equal(splashText(loading), 'Update wird heruntergeladen … 63 %')
  const installing = splashReducer(loading, { type: 'downloaded' })
  assert.deepEqual(installing, { phase: 'installing', version: '0.3.2' })
  assert.equal(splashText(installing), 'Update wird installiert …')
})

test('Splash: unbekannte Größe ohne Prozentangabe', () => {
  const s = run({ type: 'found', version: '1.0.0' }, { type: 'started', total: 0 }, { type: 'progress', bytes: 50 })
  assert.equal(splashText(s), 'Update wird heruntergeladen …')
})

test('Splash: Aufgeben (Fehler, Hänger) beendet in jeder Phase; „done“ bleibt „done“', () => {
  assert.deepEqual(run({ type: 'found', version: '1.0.0' }, { type: 'give-up' }), { phase: 'done' })
  assert.deepEqual(run({ type: 'give-up' }), { phase: 'done' })
  assert.deepEqual(run({ type: 'no-update' }, { type: 'found', version: '1.0.0' }), { phase: 'done' })
})

test('Splash: Ereignisse in falscher Phase werden ignoriert', () => {
  assert.deepEqual(run({ type: 'progress', bytes: 10 }), initialSplash)
  assert.deepEqual(run({ type: 'downloaded' }), initialSplash)
  const loading = run({ type: 'found', version: '1.0.0' })
  assert.deepEqual(splashReducer(loading, { type: 'no-update' }), loading)
  assert.deepEqual(splashReducer(loading, { type: 'found', version: '2.0.0' }), loading)
})

test('Prozent werden begrenzt und abgerundet', () => {
  assert.equal(percentOf(0, 0), null)
  assert.equal(percentOf(5, -1), null)
  assert.equal(percentOf(999, 1000), 99)
  assert.equal(percentOf(1500, 1000), 100)
  assert.equal(percentOf(1000, 1000), 100)
})

test('Hänger-Erkennung', () => {
  assert.equal(isStalled(1000, 15_999, 15_000), false)
  assert.equal(isStalled(1000, 16_000, 15_000), true)
})

test('withTimeout: Ergebnis, Fehler → null, Zeitüberschreitung → null und spätes Ergebnis an onLate', async () => {
  assert.equal(await withTimeout(Promise.resolve(7), 50), 7)
  assert.equal(await withTimeout(Promise.reject(new Error('offline')), 50), null)
  let late: string | null = null
  const slow = new Promise<string>((resolve) => setTimeout(() => resolve('spät'), 30))
  assert.equal(await withTimeout(slow, 5, (v) => (late = v)), null)
  await slow
  await new Promise((r) => setTimeout(r, 0))
  assert.equal(late, 'spät')
})

test('Hintergrund: wann heruntergeladen wird', () => {
  assert.equal(shouldDownload('idle', null, '0.3.2'), true)
  assert.equal(shouldDownload('error', '0.3.2', '0.3.2'), true)
  assert.equal(shouldDownload('downloading', '0.3.2', '0.3.3'), false)
  assert.equal(shouldDownload('installing', '0.3.2', '0.3.3'), false)
  assert.equal(shouldDownload('ready', '0.3.2', '0.3.2'), false)
  assert.equal(shouldDownload('ready', '0.3.2', '0.3.3'), true)
})

test('Sprachanruf sperrt den Neustart', () => {
  assert.equal(restartBlocked('idle'), false)
  assert.equal(restartBlocked('connecting'), true)
  assert.equal(restartBlocked('connected'), true)
  assert.equal(restartBlocked('reconnecting'), true)
})

test('Hinweis in der Nutzerleiste', () => {
  assert.equal(indicatorView('idle', null, false).visible, false)
  // Still im Hintergrund – kein Hinweis während des Downloads.
  assert.equal(indicatorView('downloading', '0.3.2', false).visible, false)

  const ready = indicatorView('ready', '0.3.2', false)
  assert.deepEqual(ready, {
    visible: true,
    label: 'Lykos 0.3.2 ist bereit – Neustart zum Aktualisieren',
    tone: 'ok',
    actionable: true,
    busy: false,
  })

  const inCall = indicatorView('ready', '0.3.2', true)
  assert.equal(inCall.actionable, false)
  assert.equal(inCall.label, 'Lykos 0.3.2 ist bereit – Neustart nach dem Sprachanruf')

  assert.equal(indicatorView('installing', '0.3.2', false).busy, true)
  assert.equal(indicatorView('installing', '0.3.2', false).actionable, false)
  assert.equal(indicatorView('error', '0.3.2', false).actionable, true)
  assert.equal(indicatorView('error', '0.3.2', true).actionable, false)
})

test('Einstellungen: Status neben „Nach Updates suchen“', () => {
  const base = {
    checking: false,
    status: 'idle' as const,
    version: null,
    progress: null,
    lastCheck: null,
    installedVersion: '0.5.0',
    inCall: false,
  }
  assert.deepEqual(updateCheckView(base), { text: '', tone: 'muted', canCheck: true, restart: 'hidden' })
  assert.equal(updateCheckView({ ...base, checking: true }).text, 'Suche …')
  assert.equal(updateCheckView({ ...base, checking: true }).canCheck, false)
  assert.equal(updateCheckView({ ...base, lastCheck: 'current' }).text, 'Lykos ist aktuell (Version 0.5.0)')
  assert.equal(updateCheckView({ ...base, lastCheck: 'failed' }).tone, 'warn')

  const loading = updateCheckView({ ...base, status: 'downloading', version: '0.6.0', progress: 42 })
  assert.equal(loading.text, 'Update 0.6.0 wird geladen … 42 %')
  assert.equal(loading.canCheck, false)
  assert.equal(updateCheckView({ ...base, status: 'downloading', version: '0.6.0' }).text, 'Update 0.6.0 wird geladen …')

  const ready = updateCheckView({ ...base, status: 'ready', version: '0.6.0', lastCheck: 'ready' })
  assert.equal(ready.text, 'Update 0.6.0 bereit')
  assert.equal(ready.restart, 'enabled')
  const inCall = updateCheckView({ ...base, status: 'ready', version: '0.6.0', inCall: true })
  assert.equal(inCall.restart, 'blocked')
  assert.equal(inCall.text, 'Update 0.6.0 bereit – Neustart nach dem Sprachanruf')

  assert.equal(updateCheckView({ ...base, status: 'installing' }).restart, 'hidden')
  assert.equal(updateCheckView({ ...base, status: 'error', version: '0.6.0' }).restart, 'enabled')
})
