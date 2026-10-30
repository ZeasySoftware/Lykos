import { useEffect, useRef } from 'react'
import type { ReactNode } from 'react'
import { X } from 'lucide-react'
import { useEscapeLayer } from '../lib/layers'

interface Props {
  title: string
  onClose: () => void
  children: ReactNode
  wide?: boolean
  /** Kleiner Bestätigungsdialog (schmaler, Text statt Formular). */
  compact?: boolean
}

export function Modal({ title, onClose, children, wide, compact }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  useEscapeLayer(true, onClose)

  // Fokus in den Dialog holen (falls kein Feld `autoFocus` hat) und beim Schließen zurückgeben.
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (!ref.current?.contains(document.activeElement)) {
      const first = ref.current?.querySelector<HTMLElement>('[autofocus], input, textarea, select, .modal-body button')
      ;(first ?? ref.current)?.focus({ preventScroll: true })
    }
    return () => {
      if (previous?.isConnected) previous.focus({ preventScroll: true })
    }
  }, [])

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={ref}
        className={`modal${wide ? ' modal-wide' : ''}${compact ? ' modal-compact' : ''}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
      >
        <header className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="icon-button" onClick={onClose} aria-label="Schließen">
            <X size={18} />
          </button>
        </header>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  )
}
