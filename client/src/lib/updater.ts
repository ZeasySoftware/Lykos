/**
 * Auto-Update der Desktop-App (Tauri-Plugin `updater`, nur im Desktop-Bundle) – Ablauf wie bei Discord:
 *
 * - **Start:** Splash „Suche nach Updates …“ prüft das Manifest (`https://lykos.zeasy.dev/api/desktop/latest.json`,
 *   fest in `tauri.conf.json`, max. 4 s). Gibt es ein Update, wird es mit Fortschritt im Splash geladen und sofort
 *   still installiert (NSIS `installMode: quiet`, kein Installer-Fenster). Das Plugin beendet die App, der Installer
 *   startet sie danach wieder (`/R`). Offline/Fehler → direkt weiter in die App.
 * - **Laufend:** alle 6 h prüfen und still im Hintergrund herunterladen (`download`, Bytes bleiben im Plugin).
 *   Danach zeigt die Nutzerleiste einen grünen Download-Pfeil; Klick → `install` + Neustart. Während eines
 *   Sprachanrufs wird nie neu gestartet.
 *
 * Das Plugin prüft die Signatur des Pakets gegen den eingebauten Public Key. Die Zustandslogik steckt testbar in
 * `updateFlow.ts`.
 */
import { create } from 'zustand'
import type { Update } from '@tauri-apps/plugin-updater'
import { isDesktopBundle } from './api'
import {
  BOOT_CHECK_TIMEOUT_MS,
  CHECK_INTERVAL_MS,
  RETRY_AFTER_FAILED_BOOT_MS,
  initialSplash,
  isStalled,
  restartBlocked,
  shouldDownload,
  splashReducer,
  withTimeout,
  type BackgroundStatus,
  type SplashEvent,
  type SplashState,
} from './updateFlow'
import { useVoice } from './voice'

/** Updater aktiv? Nur im gebauten Desktop-Bundle (nicht im Web, nicht bei `tauri dev`). */
export const updaterEnabled = isDesktopBundle && !import.meta.env.DEV

interface UpdaterStore {
  /** Start-Splash sichtbar (App noch nicht anzeigen). */
  booting: boolean
  splash: SplashState
  status: BackgroundStatus
  /** Version des (heruntergeladenen) Updates. */
  version: string | null
}

export const useUpdater = create<UpdaterStore>()(() => ({
  booting: updaterEnabled,
  splash: initialSplash,
  status: 'idle',
  version: null,
}))

/** Update, dessen Bytes im Plugin liegen (bzw. gerade geladen werden). */
let pending: Update | null = null
let lastProgressAt = 0

function splash(event: SplashEvent) {
  useUpdater.setState((s) => ({ splash: splashReducer(s.splash, event) }))
}

function finishBoot(retrySoon: boolean) {
  if (!useUpdater.getState().booting) return
  useUpdater.setState((s) => ({ booting: false, splash: splashReducer(s.splash, { type: 'give-up' }) }))
  startBackgroundChecks(retrySoon)
}

async function checkUpdate(timeoutMs?: number): Promise<Update | null> {
  const { check } = await import('@tauri-apps/plugin-updater')
  const request = check(timeoutMs ? { timeout: timeoutMs } : undefined)
  if (!timeoutMs) return request
  // Zusätzlich hart begrenzen (DNS hängt u. U. länger als das Plugin-Zeitlimit). Kommt die Antwort zu spät,
  // die Ressource im Plugin wieder freigeben.
  return withTimeout(request, timeoutMs + 500, (late) => void late?.close().catch(() => {}))
}

/**
 * Lädt `update` herunter (Bytes bleiben im Plugin) und setzt danach `ready`. `onEvent` meldet den Fortschritt
 * an den Splash, solange er sichtbar ist.
 */
async function download(update: Update, onEvent?: (e: SplashEvent) => void): Promise<boolean> {
  const previous = pending
  pending = update
  if (previous && previous !== update) void previous.close().catch(() => {})
  useUpdater.setState({ status: 'downloading', version: update.version })
  lastProgressAt = Date.now()
  try {
    await update.download((e) => {
      lastProgressAt = Date.now()
      if (e.event === 'Started') onEvent?.({ type: 'started', total: e.data.contentLength ?? 0 })
      else if (e.event === 'Progress') onEvent?.({ type: 'progress', bytes: e.data.chunkLength })
    })
    if (pending !== update) return false
    useUpdater.setState({ status: 'ready' })
    return true
  } catch {
    if (pending === update) {
      pending = null
      useUpdater.setState({ status: 'idle', version: null })
      void update.close().catch(() => {})
    }
    return false
  }
}

/** Installiert das heruntergeladene Update. Unter Windows beendet das Plugin die App; der Installer startet sie neu. */
async function installPending(): Promise<void> {
  const update = pending
  if (!update) throw new Error('kein Update heruntergeladen')
  await update.install()
  // Nur macOS/Linux kommen hier an – dort selbst neu starten.
  const { relaunch } = await import('@tauri-apps/plugin-process')
  await relaunch()
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

let bootStarted = false

/** Update-Prüfung beim Start (einmal; Splash ist bis `booting = false` sichtbar). */
export async function runStartupUpdate(): Promise<void> {
  if (!updaterEnabled || bootStarted) return
  bootStarted = true

  let update: Update | null = null
  try {
    update = await checkUpdate(BOOT_CHECK_TIMEOUT_MS)
  } catch {
    update = null
  }
  if (!update) {
    splash({ type: 'no-update' })
    // Offline/Fehler: bald im Hintergrund erneut versuchen; „kein Update“ ist davon nicht zu unterscheiden,
    // eine zusätzliche Prüfung nach einer Minute ist billig.
    finishBoot(true)
    return
  }

  splash({ type: 'found', version: update.version })
  const downloadDone = download(update, (e) => {
    if (useUpdater.getState().booting) splash(e)
  })

  // Hängt der Download, geht es ohne Update weiter – er läuft im Hintergrund weiter und meldet sich dann als „bereit“.
  const stalled = new Promise<'stalled'>((resolve) => {
    const timer = setInterval(() => {
      if (!useUpdater.getState().booting) clearInterval(timer)
      else if (isStalled(lastProgressAt, Date.now())) {
        clearInterval(timer)
        resolve('stalled')
      }
    }, 1000)
    void downloadDone.finally(() => clearInterval(timer))
  })

  const result = await Promise.race([downloadDone, stalled])
  if (result !== true) {
    finishBoot(false)
    return
  }

  splash({ type: 'downloaded' })
  useUpdater.setState({ status: 'installing' })
  try {
    await installPending()
  } catch {
    // Installer ließ sich nicht starten: App trotzdem öffnen, Hinweis in der Nutzerleiste bietet Wiederholung an.
    useUpdater.setState({ status: 'error' })
    finishBoot(false)
  }
}

// ---------------------------------------------------------------------------
// Laufende App
// ---------------------------------------------------------------------------

let backgroundStarted = false

async function backgroundCheck() {
  try {
    const update = await checkUpdate()
    if (!update) return
    const { status, version } = useUpdater.getState()
    if (!shouldDownload(status, version, update.version)) {
      void update.close().catch(() => {})
      return
    }
    await download(update)
  } catch {
    // Offline oder Server nicht erreichbar – beim nächsten Mal wieder.
  }
}

function startBackgroundChecks(retrySoon: boolean) {
  if (backgroundStarted) return
  backgroundStarted = true
  if (retrySoon) setTimeout(() => void backgroundCheck(), RETRY_AFTER_FAILED_BOOT_MS)
  setInterval(() => void backgroundCheck(), CHECK_INTERVAL_MS)
}

/** Ergebnis von „Nach Updates suchen“ (Tray-Menü). */
export type ManualCheckResult = 'disabled' | 'current' | 'downloading' | 'ready' | 'failed'

/**
 * „Nach Updates suchen“ aus dem Tray: sofort prüfen und – wie im Hintergrund – still herunterladen.
 * Danach erscheint der grüne Pfeil; neu gestartet wird weiterhin nur auf Klick.
 */
export async function checkForUpdatesNow(): Promise<ManualCheckResult> {
  if (!updaterEnabled) return 'disabled'
  const before = useUpdater.getState().status
  if (before === 'downloading' || before === 'installing') return 'downloading'
  let update: Update | null
  try {
    update = await checkUpdate()
  } catch {
    return 'failed'
  }
  if (!update) return useUpdater.getState().status === 'ready' ? 'ready' : 'current'
  const { status, version } = useUpdater.getState()
  if (!shouldDownload(status, version, update.version)) {
    void update.close().catch(() => {})
    return status === 'ready' ? 'ready' : 'downloading'
  }
  return (await download(update)) ? 'ready' : 'failed'
}

/** Klick auf den Hinweis: installieren und neu starten – nie während eines Sprachanrufs. */
export async function installUpdate(): Promise<void> {
  const { status } = useUpdater.getState()
  if (restartBlocked(useVoice.getState().status)) return
  if (status === 'error' && !pending) {
    // Bytes sind weg: neu laden, danach erscheint der Hinweis wieder.
    useUpdater.setState({ status: 'idle', version: null })
    await backgroundCheck()
    return
  }
  if (status !== 'ready' && status !== 'error') return
  useUpdater.setState({ status: 'installing' })
  try {
    await installPending()
  } catch {
    useUpdater.setState({ status: 'error' })
  }
}
