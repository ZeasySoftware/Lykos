import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronLeft, ChevronRight } from 'lucide-react'
import { useAppSettings } from '../lib/appSettings'
import { useEscapeLayer } from '../lib/layers'
import {
  edgeIndex,
  isNavigable,
  moveIndex,
  placeAbove,
  placeAtPoint,
  placeBelow,
  placeSubmenu,
  typeaheadIndex,
  visibleItems,
} from '../lib/menu'
import type { MenuItem, Point, Rect, Size } from '../lib/menu'
import type { MenuAnchor, OpenMenu } from '../lib/menuStore'
import { useMenu } from '../lib/menuStore'
import { useMutesVersion } from '../lib/notify'
import { useVoice } from '../lib/voice'
import { useStore } from '../store'

/** Verzögerung beim Öffnen/Schließen von Untermenüs per Maus (verhindert Flackern bei Diagonalbewegung). */
const SUBMENU_DELAY_MS = 120

/**
 * CSS-`zoom` am Wurzelelement (z. B. in Tests) verschiebt Koordinaten: Mausposition und
 * Element-Rechtecke kommen skaliert, `left/top` werden nochmals skaliert. Daher herausrechnen.
 */
function rootZoom(): number {
  const z = parseFloat(getComputedStyle(document.documentElement).zoom || '1')
  return Number.isFinite(z) && z > 0 ? z : 1
}

function viewport(): Size {
  const z = rootZoom()
  return { width: window.innerWidth / z, height: window.innerHeight / z }
}

function scaledRect(r: Rect): Rect {
  const z = rootZoom()
  return { left: r.left / z, top: r.top / z, right: r.right / z, bottom: r.bottom / z }
}

function rectOf(el: Element): Rect {
  const r = el.getBoundingClientRect()
  return scaledRect({ left: r.left, top: r.top, right: r.right, bottom: r.bottom })
}

function placeRoot(anchor: MenuAnchor, size: Size): Point {
  const vp = viewport()
  if (anchor.kind === 'point') {
    const z = rootZoom()
    return placeAtPoint(anchor.x / z, anchor.y / z, size, vp)
  }
  const rect = scaledRect(anchor.rect)
  return anchor.kind === 'below' ? placeBelow(rect, size, vp) : placeAbove(rect, size, vp)
}

/** Globaler Platz für das (eine) offene Kontextmenü bzw. Dropdown. In `App` einmal eingebunden. */
export function MenuHost() {
  const menu = useMenu((s) => s.menu)
  if (!menu) return null
  return createPortal(<MenuRoot key={menu.id} menu={menu} />, document.body)
}

function MenuRoot({ menu }: { menu: OpenMenu }) {
  const close = useMenu((s) => s.close)
  // Bei jeder Zustandsänderung neu aufbauen (z. B. Rollen-Haken, Lautstärke, Stummschaltung).
  useStore((s) => s)
  useVoice((s) => s.userAudio)
  useAppSettings((s) => s.developerMode)
  useMutesVersion()
  const items = visibleItems(menu.build())
  const empty = items.length === 0

  // Escape behandelt das Menü selbst (Untermenüs); der Ebenen-Eintrag sorgt nur dafür, dass
  // darunterliegende Dialoge nicht mitschließen, falls der Fokus doch außerhalb liegt.
  useEscapeLayer(true, () => close())

  useEffect(() => {
    if (empty) close(false)
  }, [empty, close])

  useEffect(() => {
    const outside = (e: Event) => {
      const t = e.target as HTMLElement | null
      if (t?.closest?.('.ctx-menu') || (t && menu.trigger?.contains(t))) return
      close(false)
    }
    const dismiss = () => close(false)
    // Per langem Druck geöffnet: Wackeln des noch aufliegenden Fingers ist kein Scrollen –
    // erst eine neue Berührung zählt.
    let touchArmed = false
    const arm = () => {
      touchArmed = true
    }
    const touchMove = (e: Event) => {
      if (touchArmed) outside(e)
    }
    document.addEventListener('mousedown', outside, true)
    // Scrollen per Rad/Touch schließt; programmatisches Scrollen (neue Nachrichten) nicht.
    document.addEventListener('wheel', outside, { capture: true, passive: true })
    document.addEventListener('touchstart', arm, { capture: true, passive: true })
    document.addEventListener('touchmove', touchMove, { capture: true, passive: true })
    window.addEventListener('resize', dismiss)
    window.addEventListener('blur', dismiss)
    return () => {
      document.removeEventListener('mousedown', outside, true)
      document.removeEventListener('wheel', outside, true)
      document.removeEventListener('touchstart', arm, true)
      document.removeEventListener('touchmove', touchMove, true)
      window.removeEventListener('resize', dismiss)
      window.removeEventListener('blur', dismiss)
    }
  }, [close, menu.trigger])

  const position = useCallback((size: Size) => placeRoot(menu.anchor, size), [menu.anchor])

  if (empty) return null
  return (
    <MenuList
      items={items}
      position={position}
      autoFocusFirst={menu.keyboard}
      onCloseAll={close}
      label={menu.label}
      minWidth={menu.minWidth}
      root
    />
  )
}

interface ListProps {
  items: MenuItem[]
  position: (size: Size) => Point
  autoFocusFirst: boolean
  onCloseAll: (restoreFocus?: boolean) => void
  /** Untermenü schließen (Pfeil links / Escape) – fehlt beim Hauptmenü. */
  onBack?: () => void
  /** Maus betritt das Untermenü (Elternmenü bricht dann sein Schließen ab). */
  onPointerEnter?: () => void
  label: string
  minWidth?: number
  root?: boolean
}

function MenuList({ items, position, autoFocusFirst, onCloseAll, onBack, onPointerEnter, label, minWidth, root }: ListProps) {
  const ref = useRef<HTMLDivElement>(null)
  const itemRefs = useRef<(HTMLElement | null)[]>([])
  const [pos, setPos] = useState<Point | null>(null)
  const [active, setActive] = useState(() => (autoFocusFirst ? edgeIndex(items, 'first') : -1))
  const [sub, setSub] = useState<{ index: number; keyboard: boolean } | null>(null)
  const timer = useRef<number | undefined>(undefined)

  // Position nach dem Messen (vor dem Zeichnen) bestimmen – auch wenn sich die Einträge ändern.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const p = position({ width: el.offsetWidth, height: el.offsetHeight })
    setPos((prev) => (prev && prev.left === p.left && prev.top === p.top ? prev : p))
  }, [position, items])

  // Fokus folgt dem markierten Eintrag; ohne Markierung liegt er auf dem Menü selbst.
  useEffect(() => {
    if (!pos) return
    const target = active >= 0 ? itemRefs.current[active] : ref.current
    const focusable = target?.matches('[data-custom]') ? target.querySelector<HTMLElement>('button, input') : target
    focusable?.focus({ preventScroll: true })
  }, [active, pos])

  useEffect(() => () => window.clearTimeout(timer.current), [])

  // Einträge können sich ändern (Rechte, Zustand) – Markierung im Rahmen halten.
  const current = active < items.length ? active : -1
  const subItem = sub ? items[sub.index] : undefined
  const subItems = subItem?.type === 'item' ? subItem.submenu : undefined

  function openSub(index: number, keyboard: boolean) {
    window.clearTimeout(timer.current)
    setActive(index)
    setSub({ index, keyboard })
  }

  function closeSub() {
    window.clearTimeout(timer.current)
    if (!sub) return
    const index = sub.index
    setSub(null)
    itemRefs.current[index]?.focus({ preventScroll: true })
  }

  function activate(index: number, keyboard: boolean) {
    const item = items[index]
    if (!item || !isNavigable(item)) return
    switch (item.type) {
      case 'item':
        if (item.submenu) {
          openSub(index, keyboard)
          return
        }
        onCloseAll()
        item.onSelect?.()
        return
      case 'checkbox':
        item.onToggle(!item.checked)
        return
      case 'radio':
        onCloseAll()
        item.onSelect()
        return
    }
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLDivElement>) {
    // Tastendrücke eines Untermenüs nicht doppelt behandeln.
    if (!ref.current?.contains(e.target as Node)) return
    const activeItem = items[current]
    const inCustom = activeItem?.type === 'custom'
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        e.preventDefault()
        setSub(null)
        setActive(moveIndex(items, current, e.key === 'ArrowDown' ? 1 : -1))
        return
      case 'Home':
      case 'End':
        if (inCustom) return
        e.preventDefault()
        setActive(edgeIndex(items, e.key === 'Home' ? 'first' : 'last'))
        return
      case 'ArrowRight':
        if (inCustom) return
        e.preventDefault()
        if (activeItem?.type === 'item' && activeItem.submenu) openSub(current, true)
        return
      case 'ArrowLeft':
        if (inCustom) return
        e.preventDefault()
        onBack?.()
        return
      case 'Escape':
        e.preventDefault()
        e.stopPropagation()
        if (onBack) onBack()
        else onCloseAll()
        return
      case 'Enter':
      case ' ':
        if (inCustom) return
        e.preventDefault()
        if (current >= 0) activate(current, true)
        return
      case 'Tab':
        e.preventDefault()
        onCloseAll()
        return
      default:
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && !inCustom) {
          const next = typeaheadIndex(items, current, e.key)
          if (next !== current) setActive(next)
        }
    }
  }

  function hover(index: number) {
    const item = items[index]
    window.clearTimeout(timer.current)
    if (!isNavigable(item)) return
    setActive(index)
    if (item.type === 'item' && item.submenu) {
      if (sub?.index !== index) timer.current = window.setTimeout(() => setSub({ index, keyboard: false }), SUBMENU_DELAY_MS)
    } else if (sub) {
      timer.current = window.setTimeout(() => setSub(null), SUBMENU_DELAY_MS * 2)
    }
  }

  const subPosition = useCallback(
    (size: Size) => {
      const el = sub ? itemRefs.current[sub.index] : null
      return el ? placeSubmenu(rectOf(el), size, viewport()) : { left: 0, top: 0 }
    },
    [sub],
  )

  return (
    <>
      <div
        ref={ref}
        className={`ctx-menu${root ? ' ctx-menu-root' : ' ctx-submenu'}`}
        role="menu"
        aria-label={label}
        tabIndex={-1}
        data-own-escape
        style={{ left: pos?.left ?? -9999, top: pos?.top ?? 0, visibility: pos ? 'visible' : 'hidden', minWidth }}
        onKeyDown={onKeyDown}
        onContextMenu={(e) => e.preventDefault()}
        onMouseEnter={() => {
          window.clearTimeout(timer.current)
          onPointerEnter?.()
        }}
        onMouseLeave={() => {
          if (!sub) setActive(-1)
        }}
      >
        {/* Mobil liegt ein Untermenü als eigenes Blatt über dem Hauptmenü – mit Weg zurück. */}
        {!root && onBack && (
          <button type="button" className="ctx-back mobile-only" onClick={onBack} tabIndex={-1}>
            <ChevronLeft size={16} aria-hidden="true" />
            <span className="truncate">{label}</span>
          </button>
        )}
        {items.map((item, i) => (
          <MenuEntry
            key={'id' in item ? item.id : `${item.type}-${i}`}
            item={item}
            active={i === current}
            expanded={sub?.index === i}
            refCallback={(el) => {
              itemRefs.current[i] = el
            }}
            onHover={() => hover(i)}
            onActivate={() => activate(i, false)}
          />
        ))}
      </div>
      {sub && subItems && subItem?.type === 'item' && (
        <MenuList
          key={sub.index}
          items={subItems}
          position={subPosition}
          autoFocusFirst={sub.keyboard}
          onCloseAll={onCloseAll}
          onBack={closeSub}
          onPointerEnter={() => {
            window.clearTimeout(timer.current)
            setActive(sub.index)
          }}
          label={subItem.label}
        />
      )}
    </>
  )
}

function MenuEntry({
  item,
  active,
  expanded,
  refCallback,
  onHover,
  onActivate,
}: {
  item: MenuItem
  active: boolean
  expanded: boolean
  refCallback: (el: HTMLElement | null) => void
  onHover: () => void
  onActivate: () => void
}) {
  switch (item.type) {
    case 'separator':
      return <div className="ctx-separator" role="separator" />
    case 'header':
      return (
        <div className="ctx-header" role="presentation">
          {item.label}
        </div>
      )
    case 'custom':
      return (
        <div
          ref={refCallback}
          className={`ctx-custom${active ? ' active' : ''}`}
          role="none"
          data-custom
          onMouseEnter={item.focusable ? onHover : undefined}
        >
          {item.render()}
        </div>
      )
    case 'item':
    case 'checkbox':
    case 'radio': {
      const classes = ['ctx-item']
      if (active) classes.push('active')
      if (item.disabled) classes.push('disabled')
      if (item.type === 'item' && item.danger) classes.push('danger')
      if (expanded) classes.push('expanded')
      const role = item.type === 'checkbox' ? 'menuitemcheckbox' : item.type === 'radio' ? 'menuitemradio' : 'menuitem'
      let trailing: ReactNode = null
      if (item.type === 'item') {
        trailing = (
          <>
            {item.hint && <kbd className="ctx-hint">{item.hint}</kbd>}
            {item.submenu ? (
              <ChevronRight size={16} className="ctx-icon" aria-hidden="true" />
            ) : (
              item.icon && (
                <span className="ctx-icon" aria-hidden="true">
                  {item.icon}
                </span>
              )
            )}
          </>
        )
      } else if (item.type === 'checkbox') {
        trailing = (
          <span className={`ctx-check${item.checked ? ' on' : ''}`} aria-hidden="true">
            {item.checked && <Check size={13} strokeWidth={3} />}
          </span>
        )
      } else {
        trailing = <span className={`ctx-radio${item.checked ? ' on' : ''}`} aria-hidden="true" />
      }
      return (
        <div
          ref={refCallback}
          className={classes.join(' ')}
          role={role}
          tabIndex={-1}
          aria-disabled={item.disabled || undefined}
          aria-checked={item.type === 'item' ? undefined : item.checked}
          aria-haspopup={item.type === 'item' && item.submenu ? 'menu' : undefined}
          aria-expanded={item.type === 'item' && item.submenu ? expanded : undefined}
          title={item.type === 'checkbox' ? item.title : undefined}
          onMouseEnter={onHover}
          onMouseMove={active ? undefined : onHover}
          onClick={(e) => {
            e.stopPropagation()
            onActivate()
          }}
        >
          {item.type === 'checkbox' && item.color && (
            <span className="role-dot" style={{ background: item.color }} aria-hidden="true" />
          )}
          {item.type === 'radio' && item.icon && (
            <span className="ctx-lead" aria-hidden="true">
              {item.icon}
            </span>
          )}
          <span className="ctx-label">
            <span className="truncate">{item.label}</span>
            {item.type === 'radio' && item.description && <span className="ctx-desc">{item.description}</span>}
          </span>
          {trailing}
        </div>
      )
    }
  }
}
