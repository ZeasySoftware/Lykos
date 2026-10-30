import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { KeyboardEvent, ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Apple, Clock, Flag, Gamepad2, Lightbulb, PawPrint, Plane, Search, Shapes, Smile } from 'lucide-react'
import type { CustomEmoji } from '../lib/api'
import { EMOJI_GROUPS, applySkin, searchCustom, searchEmojis } from '../lib/emojiSearch'
import type { UnicodeEmoji } from '../lib/emojiSearch'
import { useCustomEmojis, useEmojiData, useEmojiPrefs } from '../lib/emojiData'
import { useStore } from '../store'
import { CustomEmojiImage } from './CustomEmoji'
import { CommunityIcon } from './customize/parts'

/** Ausgewähltes Emoji. Unicode inklusive gewähltem Hautton. */
export type PickedEmoji =
  | { kind: 'unicode'; emoji: string; shortcode: string; name: string }
  | { kind: 'custom'; emoji: CustomEmoji }

export interface EmojiPickerProps {
  onPick: (emoji: PickedEmoji) => void
  onClose: () => void
  /** Custom-Emotes anbieten (Nachrichten, Reaktionen). In einzeiligen Feldern nur Unicode. */
  custom?: boolean
  /** Element, an dem der Picker ausgerichtet wird (und dessen Klicks ihn nicht schließen). Standard: Elternelement. */
  anchor?: HTMLElement | null
  /** Beschriftung für Screenreader. */
  label?: string
}

const COLS = 8
const CELL = 38
const WIDTH = 384
const HEIGHT = 440
const GROUP_ICONS = [Smile, PawPrint, Apple, Gamepad2, Plane, Lightbulb, Shapes, Flag]
const SKIN_SAMPLES = ['✋', '✋🏻', '✋🏼', '✋🏽', '✋🏾', '✋🏿']
const SKIN_LABELS = ['Standard', 'Hell', 'Mittelhell', 'Mittel', 'Mitteldunkel', 'Dunkel']

interface Item {
  key: string
  picked: PickedEmoji
  /** Für die Vorschau. */
  title: string
  shortcode: string
  render: (size: number) => ReactNode
}

interface Section {
  id: string
  label: string
  icon: ReactNode
  items: Item[]
}

function unicodeItem(e: UnicodeEmoji, skin: number): Item {
  const emoji = applySkin(e, skin)
  return {
    key: `u:${e.emoji}`,
    picked: { kind: 'unicode', emoji, shortcode: e.shortcodes[0], name: e.name },
    title: e.name,
    shortcode: e.shortcodes[0],
    render: (size) => (
      <span className="emoji-char" style={{ fontSize: size }}>
        {emoji}
      </span>
    ),
  }
}

function customItem(e: CustomEmoji): Item {
  return {
    key: `c:${e.id}`,
    picked: { kind: 'custom', emoji: e },
    title: e.name,
    shortcode: e.name,
    render: (size) => <CustomEmojiImage id={e.id} name={e.name} animated={e.animated} size={size} tooltip={false} />,
  }
}

/** Schlüssel für „Zuletzt verwendet“: Unicode ohne Hautton, Custom-Emotes per ID. */
function recentKey(p: PickedEmoji, base?: string): string {
  return p.kind === 'custom' ? `custom:${p.emoji.id}` : (base ?? p.emoji)
}

/**
 * Voller Emoji-Picker als schwebendes Fenster: Suche (deutsch/englisch), Kategorien, Custom-Emotes je Community,
 * „Zuletzt verwendet“, Hauttöne, Vorschau und Tastatursteuerung (Pfeiltasten, Enter, Esc; Fokus bleibt im
 * Suchfeld). Kategorien werden gestaffelt gerendert, damit das Öffnen schnell bleibt.
 * Shift+Klick wählt aus, ohne zu schließen.
 */
export function EmojiPicker({ onPick, onClose, custom = false, anchor, label = 'Emoji auswählen' }: EmojiPickerProps) {
  const uid = useId()
  const markerRef = useRef<HTMLSpanElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [hover, setHover] = useState<number | null>(null)
  const [skinOpen, setSkinOpen] = useState(false)
  const [rendered, setRendered] = useState(3)
  const [currentSection, setCurrentSection] = useState<string | null>(null)

  const data = useEmojiData()
  const byCommunity = useCustomEmojis()
  const communities = useStore((s) => s.communities)
  const selectedCommunityId = useStore((s) => s.selectedCommunityId)
  const { skin, recent, setSkin, addRecent } = useEmojiPrefs()

  const anchorEl = useCallback(() => anchor ?? markerRef.current?.parentElement ?? null, [anchor])

  // --- Position: über dem Anker, wenn dort Platz ist, sonst darunter; rechtsbündig, im Fenster gehalten ---
  useLayoutEffect(() => {
    const place = () => {
      const a = anchorEl()?.getBoundingClientRect()
      if (!a) return
      const w = Math.min(WIDTH, window.innerWidth - 16)
      const h = Math.min(HEIGHT, window.innerHeight - 16)
      const top = a.top - h - 8 >= 8 ? a.top - h - 8 : Math.min(a.bottom + 8, window.innerHeight - h - 8)
      const left = Math.min(Math.max(8, a.right - w), window.innerWidth - w - 8)
      setPos({ top: Math.max(8, top), left })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchorEl])

  // --- Schließen bei Klick außerhalb (Klicks auf den Anker schalten dort selbst um) und Escape ---
  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as Node
      if (panelRef.current?.contains(t) || anchorEl()?.contains(t)) return
      onClose()
    }
    document.addEventListener('mousedown', down)
    return () => document.removeEventListener('mousedown', down)
  }, [anchorEl, onClose])

  useEffect(() => {
    searchRef.current?.focus({ preventScroll: true })
    // Der Picker hängt per Portal an <body>. Klicks darin dürfen umgebende Popover (Profilkarte, Menüs), die
    // auf "mousedown außerhalb" schließen, nicht auslösen. Buttons nehmen dem Suchfeld nicht den Fokus.
    const panel = panelRef.current
    const down = (e: MouseEvent) => {
      if ((e.target as HTMLElement).closest('button')) e.preventDefault()
      e.stopPropagation()
    }
    panel?.addEventListener('mousedown', down)
    return () => panel?.removeEventListener('mousedown', down)
  }, [])

  // --- Abschnitte ---
  const customSections = useMemo<Section[]>(() => {
    if (!custom) return []
    // Aktuelle Community zuerst, dann wie in der Leiste.
    const order = [...communities].sort(
      (a, b) => Number(b.id === selectedCommunityId) - Number(a.id === selectedCommunityId),
    )
    return order
      .filter((c) => (byCommunity[c.id]?.length ?? 0) > 0)
      .map((c) => ({
        id: `community-${c.id}`,
        label: c.name,
        icon: <CommunityIcon community={c} className="emoji-nav-community" />,
        items: byCommunity[c.id].map(customItem),
      }))
  }, [custom, communities, selectedCommunityId, byCommunity])

  const sections = useMemo<Section[]>(() => {
    const byBase = new Map(data?.map((e) => [e.emoji, e]))
    const allCustom = new Map(Object.values(byCommunity).flat().map((e) => [e.id, e]))
    const recentItems = recent.flatMap((k): Item[] => {
      if (k.startsWith('custom:')) {
        const e = custom ? allCustom.get(k.slice(7)) : undefined
        return e ? [customItem(e)] : []
      }
      const e = byBase.get(k)
      return e ? [unicodeItem(e, skin)] : []
    })
    const out: Section[] = []
    if (recentItems.length > 0) {
      out.push({ id: 'recent', label: 'Zuletzt verwendet', icon: <Clock size={18} />, items: recentItems })
    }
    out.push(...customSections)
    if (data) {
      EMOJI_GROUPS.forEach((label, g) => {
        const Icon = GROUP_ICONS[g]
        out.push({
          id: `group-${g}`,
          label,
          icon: <Icon size={18} />,
          items: data.filter((e) => e.group === g).map((e) => unicodeItem(e, skin)),
        })
      })
    }
    return out
  }, [data, byCommunity, recent, custom, customSections, skin])

  const searching = query.trim().length > 0
  const results = useMemo<Section[]>(() => {
    if (!searching) return []
    const customHits = custom ? searchCustom(Object.values(byCommunity).flat(), query).map(customItem) : []
    const unicodeHits = data ? searchEmojis(data, query).map((e) => unicodeItem(e, skin)) : []
    return [{ id: 'search', label: 'Suchergebnisse', icon: <Search size={18} />, items: [...customHits, ...unicodeHits] }]
  }, [searching, custom, byCommunity, query, data, skin])

  const shown = searching ? results : sections
  const flat = useMemo(() => shown.flatMap((s) => s.items), [shown])

  // Raster-Positionen für die Pfeiltasten (jeder Abschnitt beginnt in einer neuen Zeile).
  const grid = useMemo(() => {
    const cells: { row: number; col: number }[] = []
    const rowStarts: number[] = []
    let row = 0
    let index = 0
    for (const s of shown) {
      for (let i = 0; i < s.items.length; i++) {
        if (i % COLS === 0) {
          if (i > 0) row++
          rowStarts[row] = index
        }
        cells.push({ row, col: i % COLS })
        index++
      }
      if (s.items.length > 0) row++
    }
    return { cells, rowStarts }
  }, [shown])

  // Gestaffeltes Rendern: pro Frame ein weiterer Abschnitt.
  useEffect(() => {
    if (searching || rendered >= sections.length) return
    const frame = requestAnimationFrame(() => setRendered((r) => r + 1))
    return () => cancelAnimationFrame(frame)
  }, [rendered, sections.length, searching])

  const optionId = (i: number) => `${uid}-opt-${i}`

  function moveTo(i: number) {
    if (flat.length === 0) return
    const next = Math.max(0, Math.min(flat.length - 1, i))
    setActive(next)
    setHover(null)
    setRendered(sections.length)
    requestAnimationFrame(() => document.getElementById(optionId(next))?.scrollIntoView({ block: 'nearest' }))
  }

  function moveRow(delta: number) {
    const cell = grid.cells[active]
    if (!cell) return moveTo(0)
    const row = cell.row + delta
    const start = grid.rowStarts[row]
    if (start === undefined) return
    const nextStart = grid.rowStarts[row + 1] ?? flat.length
    moveTo(Math.min(start + cell.col, nextStart - 1))
  }

  function pick(item: Item, keepOpen: boolean) {
    const base = item.key.startsWith('u:') ? item.key.slice(2) : undefined
    addRecent(recentKey(item.picked, base))
    onPick(item.picked)
    if (!keepOpen) onClose()
  }

  function onKey(e: KeyboardEvent<HTMLInputElement>) {
    const empty = query.length === 0
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        moveRow(1)
        break
      case 'ArrowUp':
        e.preventDefault()
        moveRow(-1)
        break
      case 'ArrowRight':
        if (empty || e.currentTarget.selectionStart === query.length) {
          e.preventDefault()
          moveTo(active + 1)
        }
        break
      case 'ArrowLeft':
        if (empty) {
          e.preventDefault()
          moveTo(active - 1)
        }
        break
      case 'Enter':
        e.preventDefault()
        if (flat[active]) pick(flat[active], e.shiftKey)
        break
      case 'Escape':
        e.preventDefault()
        e.stopPropagation()
        onClose()
        break
    }
  }

  function jump(id: string) {
    setRendered(sections.length)
    setQuery('')
    requestAnimationFrame(() => {
      const el = document.getElementById(`${uid}-${id}`)
      const box = scrollRef.current
      // .emoji-scroll ist positioniert, offsetTop bezieht sich also schon auf die Scrollfläche.
      if (el && box) box.scrollTo({ top: el.offsetTop })
    })
  }

  // Aktuellen Abschnitt für die Navigationsleiste merken.
  function onScroll() {
    const box = scrollRef.current
    if (!box || searching) return
    let current: string | null = null
    for (const s of sections) {
      const el = document.getElementById(`${uid}-${s.id}`)
      if (el && el.offsetTop <= box.scrollTop + 8) current = s.id
    }
    setCurrentSection(current)
  }

  const previewIndex = hover ?? active
  const preview = flat[previewIndex]
  const previewPick = preview?.picked
  const previewCommunity = previewPick?.kind === 'custom' ? previewPick.emoji.community_id : null
  const previewDetail = previewCommunity
    ? (communities.find((c) => c.id === previewCommunity)?.name ?? 'Custom-Emoji')
    : preview?.title
  let index = 0

  const panel = (
    <div
      ref={panelRef}
      className="emoji-panel"
      role="dialog"
      aria-label={label}
      style={pos ? { top: pos.top, left: pos.left } : { visibility: 'hidden' }}
    >
      <div className="emoji-panel-head">
        <label className="emoji-search">
          <Search size={15} aria-hidden="true" />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setActive(0)
              scrollRef.current?.scrollTo({ top: 0 })
            }}
            onKeyDown={onKey}
            placeholder="Emoji suchen (deutsch oder englisch)"
            aria-label="Emoji suchen"
            role="combobox"
            aria-expanded="true"
            aria-controls={`${uid}-grid`}
            aria-activedescendant={flat[active] ? optionId(active) : undefined}
            maxLength={40}
          />
        </label>
        <div className="emoji-skin">
          <button
            type="button"
            className="emoji-skin-toggle"
            title={`Hautfarbe: ${SKIN_LABELS[skin]}`}
            aria-label={`Hautfarbe wählen (aktuell ${SKIN_LABELS[skin]})`}
            aria-expanded={skinOpen}
            onClick={() => setSkinOpen((o) => !o)}
          >
            {SKIN_SAMPLES[skin]}
          </button>
          {skinOpen && (
            <div className="emoji-skin-menu" role="menu">
              {SKIN_SAMPLES.map((sample, tone) => (
                <button
                  key={tone}
                  type="button"
                  role="menuitemradio"
                  aria-checked={tone === skin}
                  className={tone === skin ? 'active' : ''}
                  title={SKIN_LABELS[tone]}
                  onClick={() => {
                    setSkin(tone)
                    setSkinOpen(false)
                  }}
                >
                  {sample}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="emoji-panel-main">
        {!searching && (
          <nav className="emoji-nav" aria-label="Kategorien">
            {sections.map((s) => (
              <button
                key={s.id}
                type="button"
                className={currentSection === s.id ? 'active' : ''}
                title={s.label}
                aria-label={s.label}
                onClick={() => jump(s.id)}
              >
                {s.icon}
              </button>
            ))}
          </nav>
        )}
        <div className="emoji-scroll" ref={scrollRef} onScroll={onScroll} id={`${uid}-grid`} role="listbox" aria-label={label}>
          {!data && !searching && sections.length === 0 && <p className="muted small emoji-empty">Lade Emojis …</p>}
          {searching && flat.length === 0 && (
            <p className="muted small emoji-empty">{data ? 'Keine Emojis gefunden.' : 'Lade Emojis …'}</p>
          )}
          {shown.map((s, si) => {
            const visible = searching || si < rendered
            const rows = Math.ceil(s.items.length / COLS)
            const start = index
            index += s.items.length
            if (s.items.length === 0) return null
            return (
              <section key={s.id} id={`${uid}-${s.id}`} className="emoji-section" aria-label={s.label}>
                {!searching && <h4 className="emoji-section-title">{s.label}</h4>}
                {visible ? (
                  <div className="emoji-grid">
                    {s.items.map((item, i) => {
                      const n = start + i
                      return (
                        <button
                          key={item.key}
                          id={optionId(n)}
                          type="button"
                          role="option"
                          aria-selected={n === active}
                          aria-label={`:${item.shortcode}:`}
                          tabIndex={-1}
                          className={`emoji-cell${n === active ? ' active' : ''}`}
                          onMouseEnter={() => setHover(n)}
                          onMouseLeave={() => setHover((h) => (h === n ? null : h))}
                          onClick={(e) => pick(item, e.shiftKey)}
                        >
                          {item.render(item.picked.kind === 'custom' ? 30 : 26)}
                        </button>
                      )
                    })}
                  </div>
                ) : (
                  <div style={{ height: rows * CELL }} />
                )}
              </section>
            )
          })}
        </div>
      </div>

      <div className="emoji-preview">
        {preview ? (
          <>
            <span className="emoji-preview-glyph">{preview.render(preview.picked.kind === 'custom' ? 32 : 28)}</span>
            <span className="emoji-preview-text">
              <strong className="truncate">:{preview.shortcode}:</strong>
              <span className="muted small truncate">
                {previewDetail}
              </span>
            </span>
          </>
        ) : (
          <span className="muted small">Wähle ein Emoji – Shift+Klick für mehrere.</span>
        )}
      </div>
    </div>
  )

  return (
    <>
      <span ref={markerRef} hidden />
      {createPortal(panel, document.body)}
    </>
  )
}
