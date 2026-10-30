import { createElement, useCallback, useLayoutEffect, useRef } from 'react'
import type { KeyboardEvent, MouseEvent } from 'react'
import { create } from 'zustand'
import { Copy } from 'lucide-react'
import type { MenuItem, Rect } from './menu'
import { copyText } from './toast'

/** Wo das Menü erscheint: am Mauszeiger (Kontextmenü) oder unter/über einem Auslöser (Dropdown). */
export type MenuAnchor =
  | { kind: 'point'; x: number; y: number }
  | { kind: 'below'; rect: Rect }
  | { kind: 'above'; rect: Rect }

export interface OpenMenu {
  id: number
  anchor: MenuAnchor
  /** Wird bei jedem Rendern neu aufgerufen – Einträge zeigen so immer den aktuellen Zustand. */
  build: () => MenuItem[]
  /** Fokus beim Schließen hierhin zurück. */
  returnFocus: HTMLElement | null
  /** Per Tastatur geöffnet → ersten Eintrag markieren. */
  keyboard: boolean
  /** Auslöser eines Dropdowns (bekommt `aria-expanded`; Klick darauf schließt). */
  trigger: HTMLElement | null
  label: string
  /** Mindestbreite (z. B. Breite des Auslösers beim Community-Dropdown). */
  minWidth?: number
}

interface MenuStore {
  menu: OpenMenu | null
  open: (menu: Omit<OpenMenu, 'id'>) => void
  /** `restoreFocus = false`, wenn die Aktion den Fokus selbst setzt (z. B. ein Dialog öffnet). */
  close: (restoreFocus?: boolean) => void
}

let nextId = 1

export const useMenu = create<MenuStore>()((set, get) => ({
  menu: null,
  open(menu) {
    get().menu?.trigger?.setAttribute('aria-expanded', 'false')
    menu.trigger?.setAttribute('aria-expanded', 'true')
    set({ menu: { ...menu, id: nextId++ } })
  },
  close(restoreFocus = true) {
    const menu = get().menu
    if (!menu) return
    menu.trigger?.setAttribute('aria-expanded', 'false')
    set({ menu: null })
    if (restoreFocus && menu.returnFocus?.isConnected) menu.returnFocus.focus({ preventScroll: true })
  },
}))

function rectOf(el: Element): Rect {
  const r = el.getBoundingClientRect()
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

/** Ausgangspunkt eines Kontextmenüs: Zielelement und ggf. markierter Text. */
export interface MenuContext {
  target: HTMLElement
  /** Markierter Text innerhalb des Elements (leer, wenn nichts markiert ist). */
  selection: string
}

/** Ist das Element ein Textfeld? Dort bleibt das Browser-Menü (Ausschneiden, Einfügen, Rechtschreibung). */
function isEditable(el: HTMLElement): boolean {
  return !!el.closest('input, textarea, select, [contenteditable="true"]')
}

function selectedTextWithin(el: HTMLElement): string {
  const sel = window.getSelection()
  if (!sel || sel.isCollapsed) return ''
  const node = sel.anchorNode
  if (!node || !el.contains(node)) return ''
  return sel.toString()
}

/** Markierten Text als „Kopieren“-Eintrag oben ergänzen. */
function withSelection(items: MenuItem[], selection: string): MenuItem[] {
  if (!selection.trim()) return items
  return [
    {
      type: 'item',
      id: 'copy-selection',
      label: 'Kopieren',
      hint: 'Strg+C',
      icon: createElement(Copy, { size: 16 }),
      onSelect: () => void copyText(selection, 'Text kopiert'),
    },
    { type: 'separator' },
    ...items,
  ]
}

/**
 * Kontextmenü für ein Element: liefert `onContextMenu` (Rechtsklick) und `onKeyDown`
 * (Kontextmenü-Taste bzw. Umschalt+F10). `build` wird erst beim Öffnen und dann bei jedem
 * Rendern des Menüs aufgerufen. Ein innerer Handler (z. B. Nutzername in einer Nachricht) gewinnt
 * gegen äußere (die Nachricht), weil er `preventDefault` setzt.
 */
export function useContextMenu(build: (ctx: MenuContext) => MenuItem[], label = 'Kontextmenü') {
  const buildRef = useRef(build)
  useLayoutEffect(() => {
    buildRef.current = build
  })
  const lastKeyboard = useRef(0)

  const openAt = useCallback(
    (target: HTMLElement, anchor: MenuAnchor, keyboard: boolean) => {
      const selection = selectedTextWithin(target)
      const ctx: MenuContext = { target, selection }
      useMenu.getState().open({
        anchor,
        build: () => withSelection(buildRef.current(ctx), selection),
        returnFocus: document.activeElement instanceof HTMLElement ? document.activeElement : null,
        keyboard,
        trigger: null,
        label,
      })
    },
    [label],
  )

  const onContextMenu = useCallback(
    (e: MouseEvent<HTMLElement>) => {
      if (e.defaultPrevented) return
      const target = e.target as HTMLElement
      if (isEditable(target)) return
      e.preventDefault()
      // Tastatur-Auslösung kommt bei manchen Browsern zusätzlich als contextmenu-Event (ohne Koordinaten).
      if (Date.now() - lastKeyboard.current < 300) return
      const fromKeyboard = e.clientX === 0 && e.clientY === 0 && e.detail === 0
      if (fromKeyboard) {
        const r = rectOf(e.currentTarget)
        openAt(target, { kind: 'point', x: r.left + 12, y: r.bottom }, true)
      } else {
        openAt(target, { kind: 'point', x: e.clientX, y: e.clientY }, false)
      }
    },
    [openAt],
  )

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      if (e.defaultPrevented) return
      if (e.key !== 'ContextMenu' && !(e.key === 'F10' && e.shiftKey)) return
      const target = e.target as HTMLElement
      if (isEditable(target)) return
      e.preventDefault()
      lastKeyboard.current = Date.now()
      const r = rectOf(e.currentTarget)
      openAt(target, { kind: 'point', x: r.left + 12, y: r.bottom }, true)
    },
    [openAt],
  )

  return { onContextMenu, onKeyDown }
}

/**
 * Dropdown an einem Button (z. B. Community-Name oben in der Kanalleiste). Liefert die Props für
 * den Auslöser: Klick öffnet/schließt, Pfeil runter/Enter/Leertaste öffnen per Tastatur.
 */
export function useDropdown(
  build: () => MenuItem[],
  opts: { label: string; placement?: 'below' | 'above'; matchWidth?: boolean },
) {
  const buildRef = useRef(build)
  useLayoutEffect(() => {
    buildRef.current = build
  })
  const { label, placement = 'below', matchWidth = false } = opts

  const toggle = useCallback(
    (trigger: HTMLElement, keyboard: boolean) => {
      const current = useMenu.getState().menu
      if (current?.trigger === trigger) {
        useMenu.getState().close()
        return
      }
      const rect = rectOf(trigger)
      useMenu.getState().open({
        anchor: placement === 'below' ? { kind: 'below', rect } : { kind: 'above', rect },
        build: () => buildRef.current(),
        returnFocus: trigger,
        keyboard,
        trigger,
        label,
        minWidth: matchWidth ? rect.right - rect.left : undefined,
      })
    },
    [label, placement, matchWidth],
  )

  return {
    'aria-haspopup': 'menu' as const,
    'aria-expanded': false,
    // Mousedown statt Klick: Der Klick-außerhalb-Handler des Menüs soll den Auslöser nicht als „außerhalb“ sehen.
    onMouseDown: (e: MouseEvent<HTMLElement>) => {
      if (e.button !== 0) return
      e.preventDefault()
      toggle(e.currentTarget, false)
    },
    onKeyDown: (e: KeyboardEvent<HTMLElement>) => {
      if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
        e.preventDefault()
        toggle(e.currentTarget, true)
      }
    },
  }
}
