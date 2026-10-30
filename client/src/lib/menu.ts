/**
 * Kontextmenüs & Dropdowns – reine Logik ohne React/DOM: Eintragstypen, Filtern (Rechte → `hidden`),
 * Positionierung mit Kollisionsvermeidung und Tastaturnavigation. Die Darstellung steckt in
 * `components/Menu.tsx`, der globale Zustand in `lib/menuStore.ts`.
 */
import type { ReactNode } from 'react'

interface Base {
  /** Ausgeblendet (z. B. fehlendes Recht) – wird von `visibleItems` entfernt. */
  hidden?: boolean
}

/** Normaler Eintrag; mit `submenu` öffnet er ein Untermenü. */
export interface ActionItem extends Base {
  type: 'item'
  id: string
  label: string
  icon?: ReactNode
  /** Tastenkürzel-Hinweis rechts (nur Anzeige). */
  hint?: string
  /** Gefährliche Aktion (rot). */
  danger?: boolean
  disabled?: boolean
  onSelect?: () => void
  submenu?: MenuItem[]
}

/** Umschalter mit Haken; das Menü bleibt nach dem Umschalten offen. */
export interface CheckboxItem extends Base {
  type: 'checkbox'
  id: string
  label: string
  checked: boolean
  disabled?: boolean
  /** Farbpunkt vor dem Text (z. B. Rollenfarbe). */
  color?: string
  /** Erklärung beim Überfahren (z. B. warum deaktiviert). */
  title?: string
  onToggle: (checked: boolean) => void
}

/** Auswahl aus einer Gruppe; schließt das Menü. */
export interface RadioItem extends Base {
  type: 'radio'
  id: string
  label: string
  /** Zweite Zeile in kleiner Schrift. */
  description?: string
  checked: boolean
  disabled?: boolean
  icon?: ReactNode
  onSelect: () => void
}

export interface SeparatorItem extends Base {
  type: 'separator'
}

export interface HeaderItem extends Base {
  type: 'header'
  label: string
}

/** Eigener Inhalt (Schnellreaktionen, Lautstärkeregler). `focusable` = per Pfeiltasten erreichbar. */
export interface CustomItem extends Base {
  type: 'custom'
  id: string
  render: () => ReactNode
  focusable?: boolean
}

export type MenuItem = ActionItem | CheckboxItem | RadioItem | SeparatorItem | HeaderItem | CustomItem

// ---------------------------------------------------------------------------
// Filtern
// ---------------------------------------------------------------------------

/**
 * Sichtbare Einträge: `hidden` entfernt, Einträge mit leerem Untermenü entfernt, Trennlinien und
 * Überschriften aufgeräumt (keine am Anfang/Ende, keine doppelten, keine Überschrift ohne Einträge).
 */
export function visibleItems(items: MenuItem[]): MenuItem[] {
  const kept: MenuItem[] = []
  for (const item of items) {
    if (item.hidden) continue
    if (item.type === 'item' && item.submenu) {
      const sub = visibleItems(item.submenu)
      if (sub.length === 0) continue
      kept.push({ ...item, submenu: sub })
      continue
    }
    kept.push(item)
  }
  const out: MenuItem[] = []
  for (let i = 0; i < kept.length; i++) {
    const item = kept[i]
    if (item.type === 'separator') {
      const prev = out[out.length - 1]
      if (!prev || prev.type === 'separator' || prev.type === 'header') continue
      out.push(item)
      continue
    }
    if (item.type === 'header') {
      // Überschrift nur, wenn bis zur nächsten Trennlinie/Überschrift ein echter Eintrag folgt.
      const next = kept[i + 1]
      if (!next || next.type === 'separator' || next.type === 'header') continue
    }
    out.push(item)
  }
  while (out.length > 0 && out[out.length - 1].type === 'separator') out.pop()
  return out
}

// ---------------------------------------------------------------------------
// Tastaturnavigation
// ---------------------------------------------------------------------------

/** Kann der Eintrag per Pfeiltasten angesteuert werden? */
export function isNavigable(item: MenuItem | undefined): boolean {
  if (!item) return false
  switch (item.type) {
    case 'item':
    case 'checkbox':
    case 'radio':
      return !item.disabled
    case 'custom':
      return !!item.focusable
    default:
      return false
  }
}

/** Erster bzw. letzter ansteuerbarer Eintrag; `-1`, wenn es keinen gibt. */
export function edgeIndex(items: MenuItem[], edge: 'first' | 'last'): number {
  if (edge === 'first') return items.findIndex((i) => isNavigable(i))
  for (let i = items.length - 1; i >= 0; i--) if (isNavigable(items[i])) return i
  return -1
}

/**
 * Nächster ansteuerbarer Eintrag in Richtung `delta` (±1), mit Umlauf. Ohne Auswahl (`current = -1`)
 * führt „runter“ zum ersten und „hoch“ zum letzten Eintrag.
 */
export function moveIndex(items: MenuItem[], current: number, delta: 1 | -1): number {
  const n = items.length
  if (n === 0) return -1
  if (current < 0 || current >= n) return edgeIndex(items, delta === 1 ? 'first' : 'last')
  for (let step = 1; step <= n; step++) {
    const i = (current + delta * step + n * step) % n
    if (isNavigable(items[i])) return i
  }
  return current
}

/** Typeahead: nächster ansteuerbarer Eintrag (nach `current`), dessen Text mit `char` beginnt. */
export function typeaheadIndex(items: MenuItem[], current: number, char: string): number {
  const c = char.toLocaleLowerCase('de')
  const n = items.length
  for (let step = 1; step <= n; step++) {
    const i = (Math.max(current, -1) + step) % n
    const item = items[i]
    if (!isNavigable(item) || !('label' in item)) continue
    if (item.label.toLocaleLowerCase('de').startsWith(c)) return i
  }
  return current
}

// ---------------------------------------------------------------------------
// Positionierung
// ---------------------------------------------------------------------------

export interface Size {
  width: number
  height: number
}
export interface Rect {
  left: number
  top: number
  right: number
  bottom: number
}
export interface Point {
  left: number
  top: number
}

/** Abstand zum Fensterrand. */
export const EDGE_MARGIN = 8

function clamp(value: number, min: number, max: number): number {
  // Passt das Menü gar nicht hinein, gewinnt der linke/obere Rand.
  return Math.max(min, Math.min(value, max))
}

/**
 * Kontextmenü am Mauszeiger: rechts unterhalb des Zeigers; stößt es rechts/unten an, wird es an der
 * jeweiligen Achse gespiegelt (links bzw. oberhalb des Zeigers) und zuletzt ins Fenster geschoben.
 */
export function placeAtPoint(x: number, y: number, size: Size, viewport: Size, margin = EDGE_MARGIN): Point {
  let left = x
  if (left + size.width > viewport.width - margin) left = x - size.width
  let top = y
  if (top + size.height > viewport.height - margin) top = y - size.height
  return {
    left: clamp(left, margin, viewport.width - size.width - margin),
    top: clamp(top, margin, viewport.height - size.height - margin),
  }
}

/**
 * Untermenü neben dem auslösenden Eintrag: rechts daneben, bei Platzmangel links; vertikal bündig mit
 * dem Eintrag (abzüglich Innenabstand), notfalls nach oben verschoben.
 */
export function placeSubmenu(item: Rect, size: Size, viewport: Size, margin = EDGE_MARGIN, gap = 4, padding = 6): Point {
  let left = item.right + gap
  if (left + size.width > viewport.width - margin) left = item.left - size.width - gap
  return {
    left: clamp(left, margin, viewport.width - size.width - margin),
    top: clamp(item.top - padding, margin, viewport.height - size.height - margin),
  }
}

/** Dropdown unter einem Auslöser (linksbündig); ohne Platz darunter oberhalb. */
export function placeBelow(anchor: Rect, size: Size, viewport: Size, margin = EDGE_MARGIN, gap = 6): Point {
  let top = anchor.bottom + gap
  if (top + size.height > viewport.height - margin && anchor.top - gap - size.height >= margin) {
    top = anchor.top - gap - size.height
  }
  return {
    left: clamp(anchor.left, margin, viewport.width - size.width - margin),
    top: clamp(top, margin, viewport.height - size.height - margin),
  }
}

/** Dropdown über einem Auslöser (z. B. Nutzerleiste unten); ohne Platz darüber darunter. */
export function placeAbove(anchor: Rect, size: Size, viewport: Size, margin = EDGE_MARGIN, gap = 6): Point {
  let top = anchor.top - gap - size.height
  if (top < margin) top = anchor.bottom + gap
  return {
    left: clamp(anchor.left, margin, viewport.width - size.width - margin),
    top: clamp(top, margin, viewport.height - size.height - margin),
  }
}
