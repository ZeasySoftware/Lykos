import { useState } from 'react'
import type { ConfirmSpec } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { Modal } from './Modal'

/**
 * Einheitliche Bestätigung für destruktive Aktionen (Löschen, Entfernen, Verlassen). Die Aktion läuft
 * im Dialog: währenddessen sind die Buttons gesperrt, ein Fehler bleibt sichtbar.
 */
export function ConfirmDialog({ spec, onClose }: { spec: ConfirmSpec; onClose: () => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const danger = spec.danger ?? true

  async function confirm() {
    setBusy(true)
    setError(null)
    try {
      await spec.action()
      onClose()
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <Modal title={spec.title} onClose={onClose} compact>
      <div className="form">
        <div className="confirm-message">{spec.message}</div>
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button ghost" onClick={onClose} disabled={busy}>
            Abbrechen
          </button>
          <button
            type="button"
            className={`button ${danger ? 'danger-solid' : 'primary'}`}
            onClick={() => void confirm()}
            disabled={busy}
            autoFocus
          >
            {spec.confirmLabel}
          </button>
        </div>
      </div>
    </Modal>
  )
}
