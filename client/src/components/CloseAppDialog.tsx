import { useState } from 'react'
import { LogOut, PhoneOff } from 'lucide-react'
import { resolveClose, useCloseAsk } from '../lib/desktop'
import { closeNeedsCallWarning, type CloseChoice } from '../lib/desktopShell'
import { toastError } from '../lib/toast'
import { useVoice } from '../lib/voice'
import { Modal } from './Modal'

/**
 * Desktop: Nachfrage beim Schließen des Fensters, solange das Verhalten nicht entschieden ist (siehe
 * `lib/desktop.ts` und desktop.rs). Escape bzw. Schließen des Dialogs = nichts tun.
 */
export function CloseAppHost() {
  const open = useCloseAsk((s) => s.open)
  return open ? <CloseAppDialog onClose={() => useCloseAsk.setState({ open: false })} /> : null
}

function CloseAppDialog({ onClose }: { onClose: () => void }) {
  const inCall = useVoice((s) => s.status !== 'idle')
  const [remember, setRemember] = useState(true)
  const [warnCall, setWarnCall] = useState(false)
  const [busy, setBusy] = useState(false)

  async function choose(choice: CloseChoice, confirmed = false) {
    if (!confirmed && closeNeedsCallWarning(choice, inCall)) {
      setWarnCall(true)
      return
    }
    setBusy(true)
    try {
      await resolveClose(choice, remember)
    } catch (e) {
      toastError(e)
      setBusy(false)
    }
  }

  if (warnCall) {
    return (
      <Modal title="Sprachanruf beenden?" onClose={onClose} compact>
        <div className="form">
          <div className="confirm-message">
            Du bist gerade in einem Sprachanruf. Wenn du Lykos beendest, wird der Anruf getrennt.
          </div>
          <div className="form-actions">
            <button type="button" className="button ghost" onClick={() => setWarnCall(false)} disabled={busy}>
              Zurück
            </button>
            <button
              type="button"
              className="button danger-solid"
              onClick={() => void choose('quit', true)}
              disabled={busy}
              autoFocus
            >
              <PhoneOff size={16} aria-hidden="true" /> Anruf beenden und schließen
            </button>
          </div>
        </div>
      </Modal>
    )
  }

  return (
    <Modal title="Lykos schließen?" onClose={onClose} compact>
      <div className="form">
        <div className="confirm-message">
          Lykos kann im Infobereich weiterlaufen – dann bleibst du erreichbar und Anrufe laufen weiter.
        </div>
        <label className="checkbox">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          Entscheidung merken
        </label>
        <p className="muted small">Ändern kannst du das jederzeit unter Einstellungen → Desktop-App.</p>
        <div className="form-actions">
          <button type="button" className="button ghost" onClick={() => void choose('quit')} disabled={busy}>
            <LogOut size={16} aria-hidden="true" /> Beenden
          </button>
          <button
            type="button"
            className="button primary"
            onClick={() => void choose('tray')}
            disabled={busy}
            autoFocus
          >
            In den Infobereich
          </button>
        </div>
      </div>
    </Modal>
  )
}
