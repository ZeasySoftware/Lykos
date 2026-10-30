/**
 * Desktop-Hülle (Tauri): eigene Titelleiste, Schließen in den Infobereich, Tray-Menü und Autostart.
 * Gegenstück zu `client/src-tauri/src/desktop.rs`.
 *
 * Erkennung über `__TAURI_INTERNALS__` statt `isDesktopBundle`: Das Fenster hat auch bei `tauri dev`
 * (Oberfläche von `localhost:5173`) keinen Rahmen und braucht die Titelleiste. Im Browser ist nichts davon aktiv.
 */
import { useAppSettings } from './appSettings'
import { isWindows11, SNAP_HOVER_DELAY_MS, trayBadge, trayTooltip, type UnreadSummary } from './desktopShell'
import { isCommunityMuted, subscribeMutes } from './notify'
import { toast } from './toast'
import { communityMentionCount, isCommunityUnread, isDmUnread } from './unread'
import { checkForUpdatesNow } from './updater'
import { useVoice } from './voice'
import { useStore } from '../store'

/** Läuft die Oberfläche in einem Tauri-Fenster (eigene Titelleiste, Tray)? */
export const hasDesktopShell = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function invoke<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(cmd, args)
}

async function currentWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  return getCurrentWindow()
}

// ---------------------------------------------------------------------------
// Fensterknöpfe
// ---------------------------------------------------------------------------

export const windowControls = {
  minimize: () => void currentWindow().then((w) => w.minimize()),
  toggleMaximize: () => void currentWindow().then((w) => w.toggleMaximize()),
  /** Geht über `CloseRequested` – je nach Einstellung ins Tray oder wirklich schließen (siehe desktop.rs). */
  close: () => void currentWindow().then((w) => w.close()),
}

/** Maximiert-Zustand beobachten (für das Symbol Maximieren/Wiederherstellen). Liefert die Abmeldung. */
export function watchMaximized(onChange: (maximized: boolean) => void): () => void {
  let stopped = false
  let unlisten: (() => void) | undefined
  void currentWindow().then(async (w) => {
    const update = () => void w.isMaximized().then((m) => !stopped && onChange(m))
    update()
    const off = await w.onResized(update)
    if (stopped) off()
    else unlisten = off
  })
  return () => {
    stopped = true
    unlisten?.()
  }
}

let snapSupported: Promise<boolean> | null = null
function supportsSnapLayouts(): Promise<boolean> {
  snapSupported ??= (async () => {
    const data = (navigator as Navigator & {
      userAgentData?: { platform?: string; getHighEntropyValues?: (hints: string[]) => Promise<{ platformVersion?: string }> }
    }).userAgentData
    if (!data?.getHighEntropyValues) return false
    try {
      const { platformVersion } = await data.getHighEntropyValues(['platformVersion'])
      return isWindows11(data.platform, platformVersion)
    } catch {
      return false
    }
  })()
  return snapSupported
}

let snapTimer: number | undefined

/** Maus über „Maximieren“: nach kurzer Verzögerung die Snap-Layouts von Windows 11 öffnen. */
export function snapHoverStart() {
  window.clearTimeout(snapTimer)
  snapTimer = window.setTimeout(() => {
    void supportsSnapLayouts().then((ok) => {
      if (ok) void invoke('desktop_snap_overlay').catch(() => {})
    })
  }, SNAP_HOVER_DELAY_MS)
}

export function snapHoverEnd() {
  window.clearTimeout(snapTimer)
}

// ---------------------------------------------------------------------------
// Autostart
// ---------------------------------------------------------------------------

export function getAutostart(): Promise<boolean> {
  return invoke<boolean>('desktop_autostart_get')
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke('desktop_autostart_set', { enabled })
}

// ---------------------------------------------------------------------------
// Tray: Zustand melden, Menü-Aktionen ausführen
// ---------------------------------------------------------------------------

function unreadSummary(): UnreadSummary {
  const s = useStore.getState()
  let mentions = 0
  let communityUnread = false
  for (const c of s.communities) {
    if (isCommunityMuted(c.id)) continue
    const detail = s.details[c.id]
    mentions += communityMentionCount(detail)
    if (!communityUnread && isCommunityUnread(detail)) communityUnread = true
  }
  let dmUnread = false
  for (const dm of Object.values(s.dms)) {
    mentions += dm.mention_count ?? 0
    // Die gerade offene DM zählt nicht, solange das Fenster sichtbar ist (wie in der Rail).
    if (!dmUnread && isDmUnread(dm) && !(dm.id === s.selectedDmId && s.dmView && document.visibilityState === 'visible'))
      dmUnread = true
  }
  return { mentions, dmUnread, communityUnread }
}

let lastTray = ''
let trayQueued = false

function syncTray() {
  if (trayQueued) return
  trayQueued = true
  // Viele Store-Änderungen (Nachrichten, Präsenz) bündeln.
  window.setTimeout(() => {
    trayQueued = false
    const v = useVoice.getState()
    const loggedIn = !!useStore.getState().user
    const summary = loggedIn ? unreadSummary() : { mentions: 0, dmUnread: false, communityUnread: false }
    const args = {
      muted: v.selfMute || v.selfDeaf,
      deafened: v.selfDeaf,
      badge: trayBadge(summary),
      tooltip: trayTooltip(summary, v.status !== 'idle'),
    }
    const key = JSON.stringify(args)
    if (key === lastTray) return
    lastTray = key
    void invoke('desktop_tray_update', args).catch(() => {
      lastTray = ''
    })
  }, 250)
}

async function onTrayAction(action: string) {
  const v = useVoice.getState()
  if (action === 'toggle-mute') v.toggleMute()
  else if (action === 'toggle-deaf') v.toggleDeaf()
  else if (action === 'check-updates') {
    toast('Suche nach Updates …')
    const result = await checkForUpdatesNow()
    const text: Record<typeof result, string> = {
      disabled: 'Updates gibt es nur in der installierten Desktop-App.',
      current: 'Lykos ist auf dem neuesten Stand.',
      downloading: 'Ein Update wird heruntergeladen …',
      ready: 'Update bereit – Klick auf den grünen Pfeil startet Lykos neu.',
      failed: 'Update-Prüfung fehlgeschlagen – bitte später erneut versuchen.',
    }
    toast(text[result])
  }
  // Haken im Menü wieder an den echten Zustand angleichen (Klick hat ihn schon umgeschaltet).
  lastTray = ''
  syncTray()
}

function syncCloseToTray(enabled: boolean) {
  void invoke('desktop_set_close_to_tray', { enabled }).catch(() => {})
}

if (hasDesktopShell) {
  document.documentElement.classList.add('has-titlebar')

  syncCloseToTray(useAppSettings.getState().closeToTray)
  useAppSettings.subscribe((s, prev) => {
    if (s.closeToTray !== prev.closeToTray) syncCloseToTray(s.closeToTray)
  })

  useStore.subscribe(syncTray)
  useVoice.subscribe((s, prev) => {
    if (s.selfMute !== prev.selfMute || s.selfDeaf !== prev.selfDeaf || s.status !== prev.status) syncTray()
  })
  subscribeMutes(syncTray)
  document.addEventListener('visibilitychange', syncTray)
  syncTray()

  void import('@tauri-apps/api/event').then(({ listen }) =>
    listen<string>('tray-action', (e) => void onTrayAction(e.payload)),
  )
}
