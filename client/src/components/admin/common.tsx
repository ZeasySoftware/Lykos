import { useState } from 'react'
import type { ReactNode } from 'react'
import { Check, ChevronLeft, ChevronRight, Copy } from 'lucide-react'
import { errorText } from '../../lib/errors'
import { Modal } from '../Modal'

/** Knopf, der einen Text in die Zwischenablage kopiert und kurz „Kopiert“ zeigt. */
export function CopyButton({ text, label = 'Kopieren' }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }
  return (
    <button type="button" className="button small-button" onClick={() => void copy()} title={text}>
      {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
      {copied ? 'Kopiert' : label}
    </button>
  )
}

/**
 * Bestätigungsdialog für destruktive Aktionen. `onConfirm` darf werfen – der Fehler wird im Dialog
 * angezeigt, sonst schließt er sich.
 */
export function ConfirmDialog({
  title,
  children,
  confirmLabel,
  danger = true,
  disabled = false,
  onConfirm,
  onClose,
}: {
  title: string
  children: ReactNode
  confirmLabel: string
  danger?: boolean
  disabled?: boolean
  onConfirm: () => Promise<void>
  onClose: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function confirm() {
    setBusy(true)
    setError(null)
    try {
      await onConfirm()
      onClose()
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <Modal title={title} onClose={onClose}>
      <div className="form">
        {children}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button
            type="button"
            className={`button ${danger ? 'danger-solid' : 'primary'}`}
            onClick={() => void confirm()}
            disabled={busy || disabled}
          >
            {busy ? 'Bitte warten …' : confirmLabel}
          </button>
        </div>
      </div>
    </Modal>
  )
}

/** Seitenwechsel für Listen (`page` ab 0). */
export function Pager({
  page,
  perPage,
  total,
  onPage,
}: {
  page: number
  perPage: number
  total: number
  onPage: (page: number) => void
}) {
  const pages = Math.max(1, Math.ceil(total / perPage))
  if (pages <= 1) return null
  return (
    <div className="ia-pager">
      <button
        type="button"
        className="icon-button small"
        disabled={page === 0}
        onClick={() => onPage(page - 1)}
        aria-label="Vorherige Seite"
      >
        <ChevronLeft size={16} />
      </button>
      <span className="muted small">
        Seite {page + 1} von {pages}
      </span>
      <button
        type="button"
        className="icon-button small"
        disabled={page + 1 >= pages}
        onClick={() => onPage(page + 1)}
        aria-label="Nächste Seite"
      >
        <ChevronRight size={16} />
      </button>
    </div>
  )
}

/** Kleine Kennzeichnung (Status, Rolle). */
export function Tag({ tone = 'neutral', children }: { tone?: 'neutral' | 'ok' | 'warn' | 'danger' | 'accent'; children: ReactNode }) {
  return <span className={`ia-tag ia-tag-${tone}`}>{children}</span>
}
