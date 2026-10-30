import { useEffect, useState } from 'react'
import { endpoints } from '../lib/api'
import type { StorageUsage } from '../lib/api'
import { errorText } from '../lib/errors'
import { formatBytes } from '../lib/uploads'

/** Balken „x von y belegt“; ohne Kontingent nur die Belegung. */
export function UsageBar({ usage }: { usage: StorageUsage }) {
  const { used_bytes: used, quota_bytes: quota } = usage
  if (quota == null) {
    return <p className="muted">{formatBytes(used)} belegt – kein Kontingent.</p>
  }
  const ratio = quota > 0 ? Math.min(1, used / quota) : 1
  const level = ratio >= 0.95 ? ' full' : ratio >= 0.8 ? ' high' : ''
  return (
    <div className="usage">
      <div
        className={`usage-bar${level}`}
        role="meter"
        aria-valuemin={0}
        aria-valuemax={quota}
        aria-valuenow={Math.min(used, quota)}
        aria-label="Belegter Speicher"
      >
        <span style={{ width: `${ratio * 100}%` }} />
      </div>
      <p className="small">
        {formatBytes(used)} von {formatBytes(quota)} belegt · {formatBytes(Math.max(0, quota - used))} frei
      </p>
    </div>
  )
}

/** Einstellungen → Speicher: eigene Belegung durch Dateianhänge. */
export function StorageTab() {
  const [usage, setUsage] = useState<StorageUsage | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    endpoints
      .myUsage()
      .then(setUsage)
      .catch((e) => setError(errorText(e)))
  }, [])

  return (
    <div className="form">
      <p className="settings-description">
        Zählt alle Dateien, die du in Nachrichten angehängt hast – auch noch nicht gesendete Uploads. Löschst du
        Nachrichten mit Anhängen, wird der Platz wieder frei. Communities können zusätzlich ein eigenes Kontingent haben.
      </p>
      {error && <p className="error">{error}</p>}
      {!usage && !error && <p className="muted">Lade …</p>}
      {usage && <UsageBar usage={usage} />}
    </div>
  )
}
