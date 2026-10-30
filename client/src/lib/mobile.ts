import { useEffect, useSyncExternalStore } from 'react'
import { create } from 'zustand'

/**
 * Mobil-Layout: zentrale Stelle für den Umbruchpunkt. CSS kann in Media-Queries keine Variablen
 * verwenden – `styles/mobile.css` nutzt denselben Wert (`max-width: 768px`). Beide zusammen ändern.
 */
export const MOBILE_MAX_WIDTH = 768
export const MOBILE_QUERY = `(max-width: ${MOBILE_MAX_WIDTH}px)`

const mql: MediaQueryList | null = typeof window !== 'undefined' && window.matchMedia ? window.matchMedia(MOBILE_QUERY) : null

/** Ist gerade das Mobil-Layout aktiv (schmales Fenster)? Außerhalb von React. */
export function isMobileLayout(): boolean {
  return mql?.matches ?? false
}

function subscribe(onChange: () => void) {
  mql?.addEventListener('change', onChange)
  return () => mql?.removeEventListener('change', onChange)
}

/** Nur wo CSS nicht reicht (andere Struktur/anderes Verhalten): `true` im Mobil-Layout. */
export function useIsMobile(): boolean {
  return useSyncExternalStore(subscribe, isMobileLayout, () => false)
}

// ---------------------------------------------------------------------------
// Schublade (Community-Rail + Kanalliste) und Vollbild-Ebenen im Mobil-Layout
// ---------------------------------------------------------------------------

interface MobileNav {
  drawerOpen: boolean
  /** Mitgliederliste als Vollbild-Ebene (Desktop: eigener, gespeicherter Schalter in `App`). */
  membersOpen: boolean
  openDrawer: () => void
  closeDrawer: () => void
  setMembersOpen: (open: boolean) => void
}

export const useMobileNav = create<MobileNav>()((set) => ({
  drawerOpen: false,
  membersOpen: false,
  openDrawer: () => set({ drawerOpen: true, membersOpen: false }),
  closeDrawer: () => set({ drawerOpen: false }),
  setMembersOpen: (open) => set({ membersOpen: open, drawerOpen: false }),
}))

/** Breite der Zone am linken Rand, in der eine Wischgeste die Schublade öffnet. */
const EDGE_PX = 24
/** Mindeststrecke für die Wischgeste. */
const SWIPE_PX = 56

/**
 * Wischgesten im Mobil-Layout: vom linken Rand nach rechts öffnet die Schublade, bei offener
 * Schublade nach links wischen schließt sie. Nur horizontale Bewegungen zählen (Scrollen bleibt).
 */
export function useDrawerSwipe(enabled: boolean) {
  useEffect(() => {
    if (!enabled) return
    let start: { x: number; y: number; open: boolean } | null = null
    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) {
        start = null
        return
      }
      const t = e.touches[0]
      const open = useMobileNav.getState().drawerOpen
      // Geschlossen: nur vom Rand. Offen: überall (Schublade oder Abdunklung).
      start = open || t.clientX <= EDGE_PX ? { x: t.clientX, y: t.clientY, open } : null
    }
    const onEnd = (e: TouchEvent) => {
      if (!start) return
      const t = e.changedTouches[0]
      const dx = t.clientX - start.x
      const dy = t.clientY - start.y
      const horizontal = Math.abs(dx) > Math.abs(dy) * 1.5
      const nav = useMobileNav.getState()
      if (horizontal && !start.open && dx > SWIPE_PX && !hasBlockingLayer()) nav.openDrawer()
      else if (horizontal && start.open && dx < -SWIPE_PX) nav.closeDrawer()
      start = null
    }
    const onCancel = () => {
      start = null
    }
    document.addEventListener('touchstart', onStart, { passive: true })
    document.addEventListener('touchend', onEnd, { passive: true })
    document.addEventListener('touchcancel', onCancel, { passive: true })
    return () => {
      document.removeEventListener('touchstart', onStart)
      document.removeEventListener('touchend', onEnd)
      document.removeEventListener('touchcancel', onCancel)
    }
  }, [enabled])
}

/** Liegt eine Ebene (Dialog, Einstellungen, Menü) über der App? Dann keine Schublade per Geste. */
function hasBlockingLayer(): boolean {
  return !!document.querySelector('.modal-backdrop, .settings-layer, .ctx-menu, .ia-layer, .lightbox')
}

// ---------------------------------------------------------------------------
// Bildschirmtastatur: sichtbare Höhe als CSS-Variable
// ---------------------------------------------------------------------------

/**
 * Hält `--app-height` und `--app-top` auf der sichtbaren Fläche (`visualViewport`). Chrome (Android)
 * verkleinert dank `interactive-widget=resizes-content` ohnehin das Layout; iOS Safari schiebt nur die
 * sichtbare Fläche – dort sorgt die Variable dafür, dass der Composer über der Tastatur bleibt.
 */
export function installViewportTracking() {
  const vv = window.visualViewport
  if (!vv) return
  const root = document.documentElement
  let frame = 0
  const update = () => {
    cancelAnimationFrame(frame)
    frame = requestAnimationFrame(() => {
      root.style.setProperty('--app-height', `${Math.round(vv.height)}px`)
      root.style.setProperty('--app-top', `${Math.round(vv.offsetTop)}px`)
    })
  }
  update()
  vv.addEventListener('resize', update)
  vv.addEventListener('scroll', update)
}

// ---------------------------------------------------------------------------
// Langer Druck → Kontextmenü
// ---------------------------------------------------------------------------

/** Ab dieser Dauer gilt eine Berührung als langer Druck. */
export const LONG_PRESS_MS = 480
/** Bewegt sich der Finger weiter, ist es Scrollen statt langer Druck. */
const MOVE_TOLERANCE_PX = 10

/**
 * Langer Druck auf Touch-Geräten öffnet das vorhandene Kontextmenü: Nach `LONG_PRESS_MS` ohne
 * Bewegung wird am berührten Element ein `contextmenu`-Ereignis ausgelöst – die bestehenden
 * `onContextMenu`-Handler (siehe `useContextMenu`) greifen also unverändert. Android löst selbst ein
 * `contextmenu` aus; dann bleibt es bei diesem einen. Der Klick nach dem Loslassen wird verschluckt.
 */
export function installLongPress() {
  let timer = 0
  let start: { x: number; y: number; target: Element } | null = null
  /** In dieser Berührung wurde bereits ein Kontextmenü geöffnet (eigenes oder vom Browser). */
  let fired = false

  const cancel = () => {
    window.clearTimeout(timer)
    timer = 0
  }

  document.addEventListener(
    'touchstart',
    (e) => {
      cancel()
      fired = false
      if (e.touches.length !== 1) {
        start = null
        return
      }
      const t = e.touches[0]
      const target = e.target instanceof Element ? e.target : null
      // Textfelder behalten das Browser-Verhalten (Auswahl, Einfügen).
      if (!target || target.closest('input, textarea, select, [contenteditable="true"]')) {
        start = null
        return
      }
      start = { x: t.clientX, y: t.clientY, target }
      timer = window.setTimeout(() => {
        timer = 0
        if (!start || fired) return
        fired = true
        const event = new MouseEvent('contextmenu', {
          bubbles: true,
          cancelable: true,
          clientX: start.x,
          clientY: start.y,
          // `detail` ≠ 0, damit `useContextMenu` es nicht für eine Tastatur-Auslösung hält.
          detail: 1,
        })
        start.target.dispatchEvent(event)
        if (event.defaultPrevented) navigator.vibrate?.(10)
        else fired = false
      }, LONG_PRESS_MS)
    },
    { passive: true },
  )

  document.addEventListener(
    'touchmove',
    (e) => {
      if (!start || !timer) return
      const t = e.touches[0]
      if (Math.hypot(t.clientX - start.x, t.clientY - start.y) > MOVE_TOLERANCE_PX) cancel()
    },
    { passive: true },
  )

  document.addEventListener(
    'touchend',
    (e) => {
      cancel()
      // Nach einem langen Druck kein Klick auf das Element darunter.
      if (fired && e.cancelable) e.preventDefault()
      start = null
    },
    { passive: false },
  )
  document.addEventListener('touchcancel', () => {
    cancel()
    start = null
  })

  // Das vom Browser selbst ausgelöste `contextmenu` (Android) ersetzt unser eigenes; ein zweites in
  // derselben Berührung wird unterdrückt.
  document.addEventListener(
    'contextmenu',
    (e) => {
      if (!e.isTrusted || !start) return
      if (fired) {
        e.preventDefault()
        e.stopPropagation()
        return
      }
      cancel()
      fired = true
    },
    true,
  )
}
