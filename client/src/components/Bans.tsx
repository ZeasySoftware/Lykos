import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { Ban, CommunityDetail } from '../lib/api'
import { confirmAction } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { toast } from '../lib/toast'
import { displayName, useStore } from '../store'
import { Modal } from './Modal'
import { Avatar } from './Presence'
import { SettingsPage } from './SettingsLayer'
import { Search } from 'lucide-react'

/** Löschzeiträume beim Bannen (Sekunden), wie bei Discord höchstens 7 Tage. */
const BAN_DELETE_OPTIONS: { seconds: number; label: string }[] = [
  { seconds: 0, label: 'Nichts löschen' },
  { seconds: 3600, label: 'Letzte Stunde' },
  { seconds: 6 * 3600, label: 'Letzte 6 Stunden' },
  { seconds: 12 * 3600, label: 'Letzte 12 Stunden' },
  { seconds: 24 * 3600, label: 'Letzte 24 Stunden' },
  { seconds: 3 * 86400, label: 'Letzte 3 Tage' },
  { seconds: 7 * 86400, label: 'Letzte 7 Tage' },
]

const REASON_MAX = 512
const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/** Mitglied bannen: optionaler Grund und Zeitraum, aus dem seine Nachrichten gelöscht werden. */
export function BanDialog({
  communityId,
  userId,
  name,
  onClose,
}: {
  communityId: string
  userId: string
  name: string
  onClose: () => void
}) {
  const communityName = useStore((s) => s.details[communityId]?.community.name ?? 'der Community')
  const loadMembers = useStore((s) => s.loadMembers)
  const [reason, setReason] = useState('')
  const [deleteSeconds, setDeleteSeconds] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await endpoints.banMember(communityId, userId, {
        reason: reason.trim() || undefined,
        delete_message_seconds: deleteSeconds,
      })
      toast(`${name} wurde gebannt.`)
      onClose()
      await loadMembers(communityId).catch(() => {})
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <Modal title={`„${name}“ bannen`} onClose={onClose}>
      <form className="form" onSubmit={submit}>
        <p className="muted small">
          {name} wird sofort aus „{communityName}“ entfernt und kann über keine Einladung zurückkommen, bis der Bann in
          den Community-Einstellungen aufgehoben wird.
        </p>
        <label className="field">
          <span>Grund (optional)</span>
          <textarea
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            maxLength={REASON_MAX}
            rows={3}
            placeholder="Nur für die Moderation sichtbar"
            autoFocus
          />
        </label>
        <label className="field">
          <span>Nachrichten löschen</span>
          <select value={deleteSeconds} onChange={(e) => setDeleteSeconds(Number(e.target.value))}>
            {BAN_DELETE_OPTIONS.map((o) => (
              <option key={o.seconds} value={o.seconds}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button ghost" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button danger-solid" disabled={busy}>
            Bannen
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Bannliste der Community: Suche, Grund, wer und wann, „Bann aufheben“ (`BAN_MEMBERS`). */
export function BansPage({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const [bans, setBans] = useState<Ban[] | null>(null)
  const [filter, setFilter] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    endpoints
      .bans(communityId)
      .then((list) => !cancelled && setBans(list))
      .catch((e) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [communityId])

  function unban(ban: Ban) {
    const name = displayName(ban.user)
    confirmAction({
      title: `Bann von „${name}“ aufheben`,
      message: `${name} kann danach über eine Einladung wieder beitreten.`,
      confirmLabel: 'Bann aufheben',
      danger: false,
      action: async () => {
        await endpoints.unbanMember(communityId, ban.user.id)
        setBans((list) => list?.filter((b) => b.user.id !== ban.user.id) ?? null)
      },
    })
  }

  const q = filter.trim().toLowerCase()
  const list = (bans ?? []).filter(
    (b) =>
      !q ||
      b.user.username.toLowerCase().includes(q) ||
      displayName(b.user).toLowerCase().includes(q) ||
      !!b.reason?.toLowerCase().includes(q),
  )

  return (
    <SettingsPage
      title={`Banns${bans ? ` – ${bans.length}` : ''}`}
      description="Gebannte können über keine Einladung beitreten. Bannen geht im Kontextmenü eines Mitglieds oder auf seiner Profilkarte."
    >
      <label className="search-input">
        <Search size={15} aria-hidden="true" />
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Name oder Grund suchen"
          aria-label="Banns durchsuchen"
        />
      </label>
      {error && <p className="error">{error}</p>}
      {!bans && !error && <p className="muted">Lade …</p>}
      {bans && bans.length === 0 && <p className="muted">Niemand ist gebannt.</p>}
      {bans && bans.length > 0 && list.length === 0 && <p className="muted">Keine Treffer.</p>}
      <ul className="admin-rows">
        {list.map((b) => (
          <li key={b.user.id} className="admin-row">
            <div className="admin-row-main">
              <Avatar user={b.user} name={displayName(b.user)} size={32} />
              <div className="admin-row-text">
                <span className="truncate">
                  <strong>{displayName(b.user)}</strong> <span className="muted small">@{b.user.username}</span>
                </span>
                <span className="ban-reason small">{b.reason ?? <span className="muted">Kein Grund angegeben</span>}</span>
                <span className="muted small">
                  {b.banned_by ? `Gebannt von ${displayName(b.banned_by)}` : 'Gebannt'} am{' '}
                  {dateFmt.format(new Date(b.created_at))}
                </span>
              </div>
              <div className="admin-row-actions">
                <button type="button" className="button small-button" onClick={() => unban(b)}>
                  Bann aufheben
                </button>
              </div>
            </div>
          </li>
        ))}
      </ul>
    </SettingsPage>
  )
}
