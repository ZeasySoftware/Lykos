// Gesammeltes, zeitlich gestreutes Neuladen nach Gateway-Events wie `PERMISSIONS_CHANGED`/`ROLE_DELETE`.
//
// Solche Events gehen an alle Mitglieder einer Community gleichzeitig. Würde jeder Client sofort
// `GET /communities/{id}` (+ Voice-States) abrufen, träfe den Server eine Lastspitze von N Anfragen in
// derselben Millisekunde – beim Umsortieren mehrerer Rollen sogar mehrmals hintereinander. Deshalb:
// - Zufallsverzögerung (Standard 0,3–2 s) verteilt die Abrufe der Clients über die Zeit,
// - weitere Events während der Wartezeit werden zusammengefasst (je ID ein Abruf),
// - jedes neue Event schiebt den Abruf noch kurz hinaus (Ruhezeit), höchstens bis `maxWaitMs`.
// Reine Logik ohne Store-Bezug; Uhr, Zufall und Timer sind für Tests injizierbar.

export interface RefreshBatchOptions {
  /** Kürzeste Zufallsverzögerung ab dem ersten Event (ms). */
  minDelayMs?: number
  /** Längste Zufallsverzögerung ab dem ersten Event (ms). */
  maxDelayMs?: number
  /** Nach jedem weiteren Event mindestens so lange Ruhe abwarten (ms). */
  quietMs?: number
  /** Spätestens so lange nach dem ersten Event wird geladen (ms). */
  maxWaitMs?: number
  now?: () => number
  random?: () => number
  setTimer?: (fn: () => void, ms: number) => unknown
  clearTimer?: (handle: unknown) => void
}

export interface RefreshBatch {
  /** ID vormerken (z. B. eine Community-ID). */
  add: (id: string) => void
  /** Vorgemerkte IDs sofort verarbeiten. */
  flush: () => void
  /** Vorgemerkte IDs verwerfen (z. B. beim Abmelden). */
  cancel: () => void
  /** Aktuell vorgemerkte IDs (für Tests/Diagnose). */
  pending: () => string[]
}

export function createRefreshBatch(run: (ids: string[]) => void, options: RefreshBatchOptions = {}): RefreshBatch {
  const {
    minDelayMs = 300,
    maxDelayMs = 2000,
    quietMs = 300,
    maxWaitMs = 5000,
    now = () => Date.now(),
    random = Math.random,
    setTimer = (fn, ms) => setTimeout(fn, ms),
    clearTimer = (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  } = options

  const ids = new Set<string>()
  let timer: unknown = null
  /** Zeitpunkt des ersten Events der laufenden Sammlung. */
  let firstAt = 0
  /** Geplanter Ausführungszeitpunkt. */
  let dueAt = 0

  function schedule(at: number) {
    if (timer !== null) clearTimer(timer)
    dueAt = at
    timer = setTimer(flush, Math.max(0, at - now()))
  }

  function flush() {
    if (timer !== null) clearTimer(timer)
    timer = null
    if (ids.size === 0) return
    const list = [...ids]
    ids.clear()
    run(list)
  }

  return {
    add(id) {
      ids.add(id)
      const t = now()
      if (timer === null) {
        firstAt = t
        schedule(t + minDelayMs + random() * (maxDelayMs - minDelayMs))
        return
      }
      // Weiteres Event: kurz Ruhe abwarten, aber nicht endlos aufschieben.
      const at = Math.min(Math.max(dueAt, t + quietMs), firstAt + maxWaitMs)
      if (at !== dueAt) schedule(at)
    },
    flush,
    cancel() {
      if (timer !== null) clearTimer(timer)
      timer = null
      ids.clear()
    },
    pending: () => [...ids],
  }
}
