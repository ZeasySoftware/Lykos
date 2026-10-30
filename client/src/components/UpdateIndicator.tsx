import { ArrowDownToLine, RefreshCw, TriangleAlert } from 'lucide-react'
import { indicatorView, restartBlocked } from '../lib/updateFlow'
import { installUpdate, updaterEnabled, useUpdater } from '../lib/updater'
import { useVoice } from '../lib/voice'

/**
 * Dezenter Update-Hinweis in der Nutzerleiste (nur Desktop-Bundle): grüner Download-Pfeil, sobald ein Update
 * im Hintergrund heruntergeladen wurde. Klick installiert und startet neu – nie während eines Sprachanrufs.
 */
export function UpdateIndicator() {
  const status = useUpdater((s) => s.status)
  const version = useUpdater((s) => s.version)
  const inCall = useVoice((s) => restartBlocked(s.status))
  if (!updaterEnabled) return null

  const view = indicatorView(status, version, inCall)
  if (!view.visible) return null

  const Icon = view.busy ? RefreshCw : view.tone === 'warn' ? TriangleAlert : ArrowDownToLine
  return (
    <button
      type="button"
      className={`icon-button update-indicator tone-${view.tone}`}
      title={view.label}
      aria-label={view.label}
      aria-disabled={!view.actionable}
      onClick={() => {
        if (view.actionable) void installUpdate()
      }}
    >
      <Icon size={18} className={view.busy ? 'spin' : undefined} />
    </button>
  )
}
