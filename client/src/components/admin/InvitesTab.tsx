import { useCallback, useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Ban, Plus } from 'lucide-react'
import { adminApi } from '../../lib/adminApi'
import type { AdminCommunity, InstanceInvite, InstanceInviteStatus } from '../../lib/adminApi'
import { errorText } from '../../lib/errors'
import { ConfirmDialog, CopyButton, Tag } from './common'
import { fmtCount, fmtDate, fmtDateTime, nameOf } from './format'

const EXPIRY = [
  { value: 24, label: '1 Tag' },
  { value: 24 * 7, label: '7 Tage' },
  { value: 24 * 30, label: '30 Tage' },
  { value: 0, label: 'Unbegrenzt' },
]
const USES = [
  { value: 1, label: '1 Person' },
  { value: 5, label: '5 Personen' },
  { value: 10, label: '10 Personen' },
  { value: 25, label: '25 Personen' },
  { value: 0, label: 'Unbegrenzt' },
]

const STATUS: Record<InstanceInviteStatus, { label: string; tone: 'ok' | 'warn' | 'danger' | 'neutral' }> = {
  active: { label: 'Aktiv', tone: 'ok' },
  expired: { label: 'Abgelaufen', tone: 'neutral' },
  used_up: { label: 'Aufgebraucht', tone: 'neutral' },
  revoked: { label: 'Widerrufen', tone: 'danger' },
}

export function InvitesTab() {
  const [invites, setInvites] = useState<InstanceInvite[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [revoking, setRevoking] = useState<InstanceInvite | null>(null)
  const [showInactive, setShowInactive] = useState(false)

  const load = useCallback(() => {
    adminApi
      .invites()
      .then((list) => {
        setInvites(list)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  useEffect(load, [load])

  const visible = invites?.filter((i) => showInactive || i.status === 'active') ?? []
  const inactiveCount = invites?.filter((i) => i.status !== 'active').length ?? 0

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Einladungen</h3>
      </div>
      <p className="muted small">
        Instanz-Einladungen erlauben die Registrierung, auch wenn die Instanz nur mit Einladung offen ist – ohne dass
        dafür eine Community nötig ist. Optional treten neue Nutzer automatisch einer Community bei.
      </p>
      <CreateInviteForm onCreated={(invite) => setInvites((list) => [invite, ...(list ?? [])])} />

      {error && <p className="error">{error}</p>}
      {invites && (
        <>
          <div className="ia-toolbar">
            <span className="muted small">
              {showInactive
                ? fmtCount(visible.length, 'Einladung', 'Einladungen')
                : fmtCount(visible.length, 'aktive Einladung', 'aktive Einladungen')}
            </span>
            {inactiveCount > 0 && (
              <label className="checkbox small">
                <input type="checkbox" checked={showInactive} onChange={(e) => setShowInactive(e.target.checked)} />
                Abgelaufene und widerrufene zeigen ({inactiveCount})
              </label>
            )}
          </div>
          {visible.length === 0 ? (
            <p className="muted">Keine Einladungen.</p>
          ) : (
            <ul className="ia-list">
              {visible.map((i) => (
                <InviteRow key={i.code} invite={i} onRevoke={() => setRevoking(i)} />
              ))}
            </ul>
          )}
        </>
      )}

      {revoking && (
        <ConfirmDialog
          title="Einladung widerrufen"
          confirmLabel="Widerrufen"
          onClose={() => setRevoking(null)}
          onConfirm={async () => {
            await adminApi.revokeInvite(revoking.code)
            load()
          }}
        >
          <p>
            Der Link <code>{revoking.code}</code> funktioniert danach nicht mehr. Bereits registrierte Konten bleiben
            bestehen.
          </p>
        </ConfirmDialog>
      )}
    </div>
  )
}

function InviteRow({ invite, onRevoke }: { invite: InstanceInvite; onRevoke: () => void }) {
  const status = STATUS[invite.status]
  return (
    <li className="ia-row">
      <div className="ia-row-main">
        <div className="ia-row-title">
          <code className="ia-code">{invite.code}</code>
          <Tag tone={status.tone}>{status.label}</Tag>
          {invite.note && <span className="truncate">{invite.note}</span>}
        </div>
        <div className="ia-row-meta muted small">
          <span>
            Genutzt {invite.uses}
            {invite.max_uses ? ` / ${invite.max_uses}` : ''}
          </span>
          <span>Läuft ab: {invite.expires_at ? fmtDateTime(invite.expires_at) : 'nie'}</span>
          <span>
            Erstellt {fmtDate(invite.created_at)} von {nameOf(invite.created_by)}
          </span>
          {invite.community && <span>Tritt bei: {invite.community.name}</span>}
        </div>
        {invite.used_by.length > 0 && (
          <div className="ia-row-meta muted small">
            <span>
              Registriert:{' '}
              {invite.used_by.map((u, idx) => (
                <span key={u.user.id} title={fmtDateTime(u.used_at)}>
                  {idx > 0 && ', '}
                  {nameOf(u.user)}
                </span>
              ))}
            </span>
          </div>
        )}
      </div>
      <div className="ia-row-actions">
        {invite.status === 'active' && (
          <>
            <CopyButton text={invite.url} label="Link kopieren" />
            <button type="button" className="button small-button danger" onClick={onRevoke}>
              <Ban size={14} aria-hidden="true" />
              Widerrufen
            </button>
          </>
        )}
      </div>
    </li>
  )
}

function CreateInviteForm({ onCreated }: { onCreated: (invite: InstanceInvite) => void }) {
  const [note, setNote] = useState('')
  const [maxUses, setMaxUses] = useState(1)
  const [expires, setExpires] = useState(24 * 7)
  const [communityId, setCommunityId] = useState('')
  const [communities, setCommunities] = useState<AdminCommunity[]>([])
  const [created, setCreated] = useState<InstanceInvite | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    adminApi
      .communities({ per_page: 100 })
      .then((page) => setCommunities([...page.items].sort((a, b) => a.name.localeCompare(b.name, 'de'))))
      .catch(() => {})
  }, [])

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const invite = await adminApi.createInvite({
        note: note.trim() || undefined,
        max_uses: maxUses || undefined,
        expires_in_hours: expires || undefined,
        community_id: communityId || undefined,
      })
      setCreated(invite)
      setNote('')
      onCreated(invite)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="ia-card form" onSubmit={submit}>
      <div className="ia-form-grid">
        <label className="field">
          <span>
            Notiz <em className="muted">(optional, nur für Admins)</em>
          </span>
          <input value={note} onChange={(e) => setNote(e.target.value)} maxLength={200} placeholder="z. B. „Für Sam“" />
        </label>
        <label className="field">
          <span>Nutzungen</span>
          <select value={maxUses} onChange={(e) => setMaxUses(Number(e.target.value))}>
            {USES.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Gültig für</span>
          <select value={expires} onChange={(e) => setExpires(Number(e.target.value))}>
            {EXPIRY.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Automatisch beitreten</span>
          <select value={communityId} onChange={(e) => setCommunityId(e.target.value)}>
            <option value="">Keiner Community</option>
            {communities.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error && <p className="error">{error}</p>}
      {created && (
        <div className="ia-created">
          <span className="muted small">Neuer Einladungslink</span>
          <div className="ia-link-row">
            <input readOnly value={created.url} onFocus={(e) => e.target.select()} aria-label="Einladungslink" />
            <CopyButton text={created.url} />
          </div>
        </div>
      )}
      <div className="form-actions">
        <button type="submit" className="button primary" disabled={busy}>
          <Plus size={16} aria-hidden="true" />
          Einladung erstellen
        </button>
      </div>
    </form>
  )
}
