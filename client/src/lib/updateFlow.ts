/**
 * Reine Zustandslogik des Desktop-Updaters (ohne Tauri, React oder Vite) – damit testbar mit `npm test`.
 *
 * Zwei Abläufe:
 * - **Start (Splash):** Update prüfen (kurzes Zeitlimit) → falls vorhanden herunterladen und sofort still
 *   installieren, bevor die App erscheint. Offline, Fehler, Zeitüberschreitung oder ein hängender Download
 *   → einfach weiter in die App.
 * - **Laufend:** periodische Prüfung lädt still im Hintergrund herunter; danach nur ein dezenter Hinweis in
 *   der Nutzerleiste. Neustart nur auf Klick und nie während eines Sprachanrufs.
 */

/** Zeitlimit für die Update-Prüfung beim Start. */
export const BOOT_CHECK_TIMEOUT_MS = 4000
/** Kommt beim Start so lange kein Download-Fortschritt, geht es ohne Update weiter (Download läuft weiter). */
export const BOOT_STALL_MS = 15_000
/** Abstand der Hintergrundprüfungen. */
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000
/** Ist die Prüfung beim Start gescheitert, wird nach dieser Zeit im Hintergrund erneut geprüft. */
export const RETRY_AFTER_FAILED_BOOT_MS = 60_000

// ---------------------------------------------------------------------------
// Start-Splash
// ---------------------------------------------------------------------------

export type SplashState =
  | { phase: 'checking' }
  | { phase: 'downloading'; version: string; downloaded: number; total: number }
  | { phase: 'installing'; version: string }
  /** Splash fertig – die App wird angezeigt. */
  | { phase: 'done' }

export type SplashEvent =
  /** Kein Update, offline, Fehler oder Zeitüberschreitung bei der Prüfung. */
  | { type: 'no-update' }
  | { type: 'found'; version: string }
  | { type: 'started'; total: number }
  | { type: 'progress'; bytes: number }
  | { type: 'downloaded' }
  /** Download fehlgeschlagen, hängt oder Installation ließ sich nicht starten. */
  | { type: 'give-up' }

export const initialSplash: SplashState = { phase: 'checking' }

export function splashReducer(state: SplashState, event: SplashEvent): SplashState {
  if (state.phase === 'done') return state
  switch (event.type) {
    case 'no-update':
      return state.phase === 'checking' ? { phase: 'done' } : state
    case 'found':
      return state.phase === 'checking'
        ? { phase: 'downloading', version: event.version, downloaded: 0, total: 0 }
        : state
    case 'started':
      return state.phase === 'downloading' ? { ...state, total: Math.max(0, event.total) } : state
    case 'progress':
      return state.phase === 'downloading'
        ? { ...state, downloaded: state.downloaded + Math.max(0, event.bytes) }
        : state
    case 'downloaded':
      return state.phase === 'downloading' ? { phase: 'installing', version: state.version } : state
    case 'give-up':
      return { phase: 'done' }
  }
}

/** Fortschritt in ganzen Prozent; `null`, wenn die Gesamtgröße unbekannt ist. */
export function percentOf(downloaded: number, total: number): number | null {
  if (!(total > 0)) return null
  return Math.max(0, Math.min(100, Math.floor((downloaded / total) * 100)))
}

/** Statuszeile im Splash. */
export function splashText(state: SplashState): string {
  switch (state.phase) {
    case 'checking':
      return 'Suche nach Updates …'
    case 'downloading': {
      const percent = percentOf(state.downloaded, state.total)
      return percent === null ? 'Update wird heruntergeladen …' : `Update wird heruntergeladen … ${percent} %`
    }
    case 'installing':
      return 'Update wird installiert …'
    case 'done':
      return 'Lykos wird geladen …'
  }
}

/** Hängt der Download (seit `limitMs` kein Fortschritt)? */
export function isStalled(lastProgressAt: number, now: number, limitMs = BOOT_STALL_MS): boolean {
  return now - lastProgressAt >= limitMs
}

/** Fehler und Zeitüberschreitung gleich behandeln: Ergebnis oder `null` (nie eine Ausnahme). */
export function withTimeout<T>(promise: Promise<T>, ms: number, onLate?: (value: T) => void): Promise<T | null> {
  return new Promise((resolve) => {
    let settled = false
    const timer = setTimeout(() => {
      settled = true
      resolve(null)
    }, ms)
    promise.then(
      (value) => {
        if (settled) {
          onLate?.(value)
          return
        }
        settled = true
        clearTimeout(timer)
        resolve(value)
      },
      () => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        resolve(null)
      },
    )
  })
}

// ---------------------------------------------------------------------------
// Laufende App: Hintergrund-Download und Hinweis
// ---------------------------------------------------------------------------

export type BackgroundStatus =
  | 'idle'
  /** Lädt still im Hintergrund. */
  | 'downloading'
  /** Heruntergeladen, Installation auf Klick. */
  | 'ready'
  | 'installing'
  /** Installation ließ sich nicht starten – Klick versucht es neu. */
  | 'error'

/** Soll eine bei der Prüfung gefundene Version heruntergeladen werden? */
export function shouldDownload(status: BackgroundStatus, readyVersion: string | null, found: string): boolean {
  if (status === 'downloading' || status === 'installing') return false
  // Schon bereit: nur eine noch neuere Version ersetzt das heruntergeladene Paket.
  if (status === 'ready') return readyVersion !== found
  return true
}

/** Sprachanruf aktiv (auch beim Verbinden/Wiederverbinden)? Dann wird nie neu gestartet. */
export function restartBlocked(voiceStatus: string): boolean {
  return voiceStatus !== 'idle'
}

export interface IndicatorView {
  visible: boolean
  label: string
  tone: 'ok' | 'muted' | 'warn'
  /** Klick installiert bzw. versucht es erneut. */
  actionable: boolean
  busy: boolean
}

/** Darstellung des Update-Hinweises in der Nutzerleiste. */
export function indicatorView(status: BackgroundStatus, version: string | null, inCall: boolean): IndicatorView {
  const name = version ? `Lykos ${version}` : 'Das Update'
  switch (status) {
    case 'idle':
    case 'downloading':
      return { visible: false, label: '', tone: 'muted', actionable: false, busy: false }
    case 'ready':
      return inCall
        ? {
            visible: true,
            label: `${name} ist bereit – Neustart nach dem Sprachanruf`,
            tone: 'muted',
            actionable: false,
            busy: false,
          }
        : {
            visible: true,
            label: `${name} ist bereit – Neustart zum Aktualisieren`,
            tone: 'ok',
            actionable: true,
            busy: false,
          }
    case 'installing':
      return { visible: true, label: 'Update wird installiert – Lykos startet neu …', tone: 'ok', actionable: false, busy: true }
    case 'error':
      return {
        visible: true,
        label: inCall
          ? 'Update fehlgeschlagen – nach dem Sprachanruf erneut versuchen'
          : 'Update fehlgeschlagen – erneut versuchen',
        tone: 'warn',
        actionable: !inCall,
        busy: false,
      }
  }
}
