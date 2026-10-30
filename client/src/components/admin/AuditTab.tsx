import { useEffect, useState } from 'react'
import { adminApi } from '../../lib/adminApi'
import type { AuditEntry } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { fmtDateTime, nameOf } from './format'

const MODE_LABELS: Record<string, string> = { open: 'offen', invite_only: 'nur mit Einladung', closed: 'geschlossen' }

const SETTING_LABELS: Record<string, string> = {
  instance_name: 'Name',
  registration_mode: 'Registrierung',
  max_upload_mb: 'Upload-Größe',
  user_quota_mb: 'Speicher pro Nutzer',
  allow_community_creation: 'Communities anlegen',
  announcement: 'Ankündigung',
}

function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

/** Lesbare Beschreibung eines Eintrags im instanzweiten Audit-Log. */
function describe(e: AuditEntry): string {
  const d = e.data
  const user = str(d.username) ? `@${str(d.username)}` : 'ein Konto'
  switch (e.action) {
    case 'instance_invite.create':
      return `hat die Einladung ${str(d.code)} erstellt${str(d.note) ? ` („${str(d.note)}“)` : ''}`
    case 'instance_invite.revoke':
      return `hat die Einladung ${str(d.code)} widerrufen`
    case 'user.disable':
      return `hat ${user} gesperrt`
    case 'user.enable':
      return `hat die Sperre von ${user} aufgehoben`
    case 'user.sessions_revoke':
      return `hat alle Sitzungen von ${user} beendet`
    case 'user.admin_grant':
      return `hat ${user} zum Instanz-Admin gemacht`
    case 'user.admin_revoke':
      return `hat ${user} das Admin-Recht entzogen`
    case 'user.password_reset_link':
      return `hat einen Passwort-Reset-Link für ${user} erzeugt`
    case 'user.delete':
      return `hat das Konto ${user} gelöscht${d.delete_messages ? ' (inkl. Nachrichten)' : ''}`
    case 'community.delete':
      return `hat die Community „${str(d.name)}“ gelöscht`
    case 'community.transfer':
      return `hat den Besitz von „${str(d.name)}“ übertragen`
    case 'feedback.status':
      return `hat den Status einer Meldung auf „${str(d.status)}“ gesetzt`
    case 'feedback.reply':
      return d.internal ? 'hat eine interne Notiz zu einer Meldung geschrieben' : 'hat auf eine Meldung geantwortet'
    case 'feedback.delete':
      return 'hat eine Meldung gelöscht'
    case 'feedback.issue':
      return 'hat aus einer Meldung ein Gitea-Issue erstellt'
    case 'settings.update': {
      const parts = Object.entries(d).map(([k, v]) => {
        const label = SETTING_LABELS[k] ?? k
        if (v === null) return `${label} zurückgesetzt`
        if (k === 'registration_mode') return `${label}: ${MODE_LABELS[str(v)] ?? str(v)}`
        if (k === 'user_quota_mb' && v === 0) return `${label}: unbegrenzt`
        if (typeof v === 'boolean') return `${label}: ${v ? 'ja' : 'nein'}`
        if (k === 'announcement') return `${label} geändert`
        return `${label}: ${String(v)}`
      })
      return `hat Einstellungen geändert (${parts.join(', ')})`
    }
    default:
      return e.action
  }
}

export function AuditTab() {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null)
  const [more, setMore] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    adminApi
      .auditLog()
      .then((list) => {
        setEntries(list)
        setMore(list.length === 50)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  async function loadMore() {
    if (!entries?.length) return
    try {
      const next = await adminApi.auditLog(entries[entries.length - 1].id)
      setEntries([...entries, ...next])
      setMore(next.length === 50)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Audit-Log</h3>
      </div>
      <p className="muted small">Alle Aktionen der Instanz-Verwaltung, neueste zuerst.</p>
      {error && <p className="error">{error}</p>}
      {entries &&
        (entries.length === 0 ? (
          <p className="muted">Noch keine Einträge.</p>
        ) : (
          <ul className="audit-list">
            {entries.map((e) => (
              <li key={e.id} className="audit-entry">
                <span>
                  <strong>{nameOf(e.actor)}</strong> {describe(e)}
                </span>
                <time dateTime={e.created_at}>{fmtDateTime(e.created_at)}</time>
              </li>
            ))}
          </ul>
        ))}
      {more && (
        <button type="button" className="button small-button" onClick={() => void loadMore()}>
          Ältere laden
        </button>
      )}
    </div>
  )
}
