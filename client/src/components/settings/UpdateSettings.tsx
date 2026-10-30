import { RefreshCw, RotateCw } from 'lucide-react'
import { restartBlocked, updateCheckView } from '../../lib/updateFlow'
import { checkForUpdatesNow, installUpdate, useUpdater } from '../../lib/updater'
import { useVoice } from '../../lib/voice'

/**
 * Einstellungen „Desktop-App“ → Updates (nur Desktop-Bundle): installierte Version, „Nach Updates suchen“
 * (derselbe Weg wie im Tray-Menü) und der Status daneben; ein fertiges Update startet per „Jetzt neu starten“ –
 * nie während eines Sprachanrufs, wie beim grünen Pfeil in der Nutzerleiste.
 */
export function UpdateSettings() {
  const checking = useUpdater((s) => s.checking)
  const status = useUpdater((s) => s.status)
  const version = useUpdater((s) => s.version)
  const progress = useUpdater((s) => s.progress)
  const lastCheck = useUpdater((s) => s.lastCheck)
  const inCall = useVoice((s) => restartBlocked(s.status))
  const view = updateCheckView({
    checking,
    status,
    version,
    progress,
    lastCheck,
    installedVersion: __APP_VERSION__,
    inCall,
  })
  const busy = checking || status === 'downloading' || status === 'installing'

  return (
    <div className="update-check">
      <p className="muted small">Installierte Version: Lykos {__APP_VERSION__}</p>
      <div className="update-check-row">
        <button
          type="button"
          className="button small-button"
          disabled={!view.canCheck}
          onClick={() => void checkForUpdatesNow()}
        >
          <RefreshCw size={14} aria-hidden="true" className={busy ? 'spin' : undefined} /> Nach Updates suchen
        </button>
        {view.restart !== 'hidden' && (
          <button
            type="button"
            className="button primary small-button"
            disabled={view.restart === 'blocked'}
            title={view.restart === 'blocked' ? 'Während eines Sprachanrufs startet Lykos nicht neu.' : undefined}
            onClick={() => void installUpdate()}
          >
            <RotateCw size={14} aria-hidden="true" /> Jetzt neu starten
          </button>
        )}
        {view.text && (
          <span className={`update-check-status tone-${view.tone}`} role="status">
            {view.text}
          </span>
        )}
      </div>
    </div>
  )
}
