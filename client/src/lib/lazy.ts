// Code-Splitting selten genutzter, großer Bereiche (Einstellungen, Community-/Kanal-Einstellungen,
// Instanz-Verwaltung, Feedback, Call-Bühne): Sie landen in eigenen Chunks und werden erst geladen, wenn
// man sie öffnet – oder vorher im Leerlauf nach dem Start (`preloadLazyPanels`), damit das Öffnen dann
// ohne Ladepause und ohne Flackern geht.
import { lazy } from 'react'
import type { ComponentType, LazyExoticComponent } from 'react'

const preloads: (() => Promise<unknown>)[] = []

/**
 * Wie `React.lazy`, aber für benannte Exporte: `lazyComponent(() => import('./X').then((m) => m.X))`.
 * Der Import läuft höchstens einmal (Vorladen und erstes Rendern teilen sich das Promise).
 */
// `any` wie bei `React.lazy` selbst: die Props kommen über `C` durch.
export function lazyComponent<C extends ComponentType<any>>(load: () => Promise<C>): LazyExoticComponent<C> {
  let promise: Promise<C> | null = null
  const once = () => (promise ??= load())
  preloads.push(once)
  return lazy(() => once().then((component) => ({ default: component })))
}

let preloaded = false

/** Alle Lazy-Bereiche im Leerlauf vorladen (einmal pro Sitzung; Fehler sind egal – dann beim Öffnen). */
export function preloadLazyPanels() {
  if (preloaded) return
  preloaded = true
  const run = () => {
    for (const load of preloads) load().catch(() => {})
  }
  if (typeof requestIdleCallback === 'function') requestIdleCallback(run, { timeout: 5000 })
  else setTimeout(run, 2000)
}
