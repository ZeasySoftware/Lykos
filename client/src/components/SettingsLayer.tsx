import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { ChevronLeft, X } from 'lucide-react'
import { useIsMobile } from '../lib/mobile'
import { useEscapeLayer } from '../lib/layers'
import { UnsavedContext } from '../lib/unsaved'
import type { PendingChanges } from '../lib/unsaved'

export interface SettingsSection<T extends string> {
  id: T
  label: string
  icon?: ReactNode
  /** Rot (Gefahrenzone, Abmelden). */
  danger?: boolean
  hidden?: boolean
}

export interface SettingsGroup<T extends string> {
  /** Kategorie-Überschrift (klein, Großbuchstaben). */
  title?: string
  items: SettingsSection<T>[]
}

/** Dauer der Ausblend-Animation (muss zu `settings-layer-out` in settings.css passen). */
const EXIT_MS = 160

function prefersReducedMotion(): boolean {
  return (
    document.documentElement.classList.contains('reduce-motion') ||
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
  )
}

/**
 * Einstellungen als Vollbild-Ebene (wie Discord): links gruppierte Bereiche, rechts der Inhalt
 * (max. 740 px), oben rechts „ESC“. Ungespeicherte Änderungen (siehe `useUnsavedChanges`) zeigen
 * unten eine Leiste und halten Schließen/Bereichswechsel auf – die Leiste blinkt dann kurz.
 */
export function SettingsLayer<T extends string>({
  label,
  groups,
  active,
  onSelect,
  onClose,
  onAction,
  sidebarFooter,
  children,
}: {
  label: string
  groups: SettingsGroup<T>[]
  active: T
  onSelect: (id: T) => void
  onClose: () => void
  /** Einträge, die keinen Bereich öffnen, sondern etwas tun (z. B. „Abmelden“). */
  onAction?: (id: T) => boolean
  sidebarFooter?: ReactNode
  children: ReactNode
}) {
  const forms = useRef(new Map<object, PendingChanges>())
  const [dirty, setDirty] = useState(false)
  const [busy, setBusy] = useState(false)
  const [nag, setNag] = useState(0)
  const [closing, setClosing] = useState(false)
  const contentRef = useRef<HTMLDivElement>(null)
  const layerRef = useRef<HTMLDivElement>(null)
  // Mobil: erst die Liste der Bereiche, ein Tippen öffnet die Unterseite (mit „Zurück“).
  const mobile = useIsMobile()
  const [pageOpen, setPageOpen] = useState(false)
  const activeLabel = groups.flatMap((g) => g.items).find((i) => i.id === active)?.label ?? label

  const report = useCallback((form: object, pending: PendingChanges | null) => {
    if (pending) forms.current.set(form, pending)
    else forms.current.delete(form)
    const list = [...forms.current.values()]
    setDirty(list.some((p) => p.dirty))
    setBusy(list.some((p) => p.busy))
  }, [])

  /** Hinweis auf ungespeicherte Änderungen: Leiste kurz hervorheben. */
  const blocked = useCallback(() => {
    if (forms.current.size === 0) return false
    setNag((n) => n + 1)
    return true
  }, [])

  const api = useMemo(() => ({ report, blocked }), [report, blocked])

  const requestClose = useCallback(() => {
    if (blocked() || closing) return
    if (prefersReducedMotion()) {
      onClose()
      return
    }
    setClosing(true)
    window.setTimeout(onClose, EXIT_MS)
  }, [blocked, closing, onClose])

  /** Mobil von der Unterseite zurück zur Liste (ungespeicherte Änderungen halten auf). */
  const back = useCallback(() => {
    if (blocked()) return
    setPageOpen(false)
  }, [blocked])

  useEscapeLayer(true, mobile && pageOpen ? back : requestClose)

  useEffect(() => {
    if (!nag) return
    const t = window.setTimeout(() => setNag(0), 900)
    return () => window.clearTimeout(t)
  }, [nag])

  // Fokus in die Ebene holen, beim Schließen zurückgeben.
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    layerRef.current?.querySelector<HTMLElement>('.settings-nav-item.active')?.focus({ preventScroll: true })
    return () => {
      if (previous?.isConnected) previous.focus({ preventScroll: true })
    }
  }, [])

  function select(id: T) {
    if (onAction?.(id)) return
    if (id === active) {
      setPageOpen(true)
      return
    }
    if (blocked()) return
    onSelect(id)
    setPageOpen(true)
    contentRef.current?.scrollTo({ top: 0 })
  }

  async function saveAll() {
    for (const p of [...forms.current.values()]) await p.save()
  }

  function resetAll() {
    for (const p of [...forms.current.values()]) p.reset()
  }

  return (
    <div
      ref={layerRef}
      className={`settings-layer${closing ? ' closing' : ''}${pageOpen ? ' mobile-page' : ' mobile-list'}`}
      role="dialog"
      aria-modal="true"
      aria-label={label}
    >
      <header className="settings-mobile-bar mobile-only">
        {pageOpen ? (
          <button type="button" className="icon-button" onClick={back} aria-label="Zurück">
            <ChevronLeft size={22} />
          </button>
        ) : null}
        <h2 className="truncate">{pageOpen ? activeLabel : label}</h2>
        <button type="button" className="icon-button" onClick={requestClose} aria-label="Schließen">
          <X size={20} />
        </button>
      </header>
      <aside className="settings-sidebar">
        <nav className="settings-sidebar-inner" aria-label={label}>
          {groups.map((g, gi) => {
            const items = g.items.filter((i) => !i.hidden)
            if (items.length === 0) return null
            return (
              <div key={g.title ?? gi} className="settings-nav-group">
                {g.title && <h4 className="settings-nav-title">{g.title}</h4>}
                {items.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`settings-nav-item${item.id === active ? ' active' : ''}${item.danger ? ' danger' : ''}`}
                    aria-current={item.id === active ? 'page' : undefined}
                    onClick={() => select(item.id)}
                  >
                    <span className="truncate">{item.label}</span>
                    {item.icon && (
                      <span className="settings-nav-icon" aria-hidden="true">
                        {item.icon}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )
          })}
          {sidebarFooter}
        </nav>
      </aside>
      <div className="settings-main" ref={contentRef}>
        <div className="settings-content">
          <UnsavedContext.Provider value={api}>{children}</UnsavedContext.Provider>
        </div>
        <div className="settings-tools">
          <button type="button" className="settings-close" onClick={requestClose} aria-label="Schließen">
            <X size={18} />
          </button>
          <span className="settings-close-hint" aria-hidden="true">
            ESC
          </span>
        </div>
        {dirty && (
          <div className="unsaved-anchor">
            <div className={`unsaved-bar${nag ? ' nag' : ''}`} role="status" key={nag || undefined}>
              <span>Achtung – du hast ungespeicherte Änderungen!</span>
              <span className="unsaved-actions">
                <button type="button" className="button ghost small-button" onClick={resetAll} disabled={busy}>
                  Zurücksetzen
                </button>
                <button
                  type="button"
                  className="button primary small-button"
                  onClick={() => void saveAll()}
                  disabled={busy}
                >
                  {busy ? 'Speichert …' : 'Änderungen speichern'}
                </button>
              </span>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

/** Seitenkopf eines Bereichs: Titel und optionale Beschreibung. */
export function SettingsPage({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) {
  return (
    <section className="settings-page">
      <header className="settings-page-header">
        <h2>{title}</h2>
        {description && <p className="settings-description">{description}</p>}
      </header>
      {children}
    </section>
  )
}

/** Abschnitt mit kleiner Überschrift, Beschreibung und Trennlinie davor. */
export function SettingsSection({
  title,
  description,
  danger,
  children,
}: {
  title: string
  description?: ReactNode
  danger?: boolean
  children?: ReactNode
}) {
  return (
    <section className={`settings-section${danger ? ' danger' : ''}`}>
      <h3 className="settings-section-title">{title}</h3>
      {description && <p className="settings-description">{description}</p>}
      {children}
    </section>
  )
}
