import { useEffect, useRef } from 'react'
import type { RefObject } from 'react'

/**
 * Ebenen-Stapel für Escape: Dialoge, Einstellungen, Popover und Menüs melden sich hier an. Escape
 * schließt immer nur die oberste (zuletzt geöffnete) Ebene – nicht alle gleichzeitig.
 * Elemente mit `data-own-escape` (z. B. Menüs mit Untermenüs) behandeln Escape selbst.
 */
interface Layer {
  id: number
  onEscape: () => void
}

const stack: Layer[] = []
let nextId = 1
/** Während z. B. eine Taste belegt wird, gehört Escape dem Formular. */
let suspended = false

export function suspendEscape(value: boolean) {
  suspended = value
}

window.addEventListener(
  'keydown',
  (e) => {
    if (e.key !== 'Escape' || stack.length === 0 || suspended) return
    const target = e.target as HTMLElement | null
    if (target?.closest?.('[data-own-escape]')) return
    e.preventDefault()
    e.stopPropagation()
    stack[stack.length - 1].onEscape()
  },
  true,
)

/** Gibt es gerade eine offene Ebene (Dialog, Menü, Popover)? */
export function hasOpenLayer(): boolean {
  return stack.length > 0
}

/** Meldet eine Ebene an, solange `active` gilt; Escape ruft dann `onEscape` (nur für die oberste). */
export function useEscapeLayer(active: boolean, onEscape: () => void) {
  const handler = useRef(onEscape)
  useEffect(() => {
    handler.current = onEscape
  })
  useEffect(() => {
    if (!active) return
    const layer: Layer = { id: nextId++, onEscape: () => handler.current() }
    stack.push(layer)
    return () => {
      const i = stack.findIndex((l) => l.id === layer.id)
      if (i >= 0) stack.splice(i, 1)
    }
  }, [active])
}

/**
 * Popover-Verhalten: Klick außerhalb von `ref` oder Escape schließt. `ignore` (optional): Klicks auf
 * dieses Element (z. B. den Auslöser-Button) zählen nicht als „außerhalb“ – der Button schaltet selbst um.
 */
export function useDismiss(
  ref: RefObject<HTMLElement | null>,
  open: boolean,
  onClose: () => void,
  ignore?: RefObject<HTMLElement | null>,
) {
  useEscapeLayer(open, onClose)
  useEffect(() => {
    if (!open) return
    const click = (e: MouseEvent) => {
      const target = e.target as Node
      if (ref.current?.contains(target) || ignore?.current?.contains(target)) return
      // Klicks in Menüs/Dialogen, die über dem Popover liegen, schließen es nicht.
      if ((target as HTMLElement).closest?.('.ctx-menu, .modal-backdrop')) return
      onClose()
    }
    document.addEventListener('mousedown', click)
    return () => document.removeEventListener('mousedown', click)
  }, [ref, ignore, open, onClose])
}
