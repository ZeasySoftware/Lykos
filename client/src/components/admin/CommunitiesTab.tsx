import { useEffect, useState } from 'react'
import { Crown, Search, Trash2 } from 'lucide-react'
import { adminApi } from '../../lib/adminApi'
import type { AdminCommunity, Page } from '../../lib/adminApi'
import type { UserPublic } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { formatBytes } from '../../lib/uploads'
import { ConfirmDialog, Pager } from './common'
import { fmtDate, fmtNumber, nameOf } from './format'

const PER_PAGE = 25

export function CommunitiesTab() {
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  const [data, setData] = useState<Page<AdminCommunity> | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)
  const [deleting, setDeleting] = useState<AdminCommunity | null>(null)
  const [transferring, setTransferring] = useState<AdminCommunity | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      adminApi
        .communities({ q: query.trim() || undefined, page, per_page: PER_PAGE })
        .then((d) => {
          if (cancelled) return
          setData(d)
          setError(null)
        })
        .catch((e) => !cancelled && setError(errorText(e)))
    }, 200)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [query, page, reload])

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Communities</h3>
        {data && <span className="muted small">{fmtNumber(data.total)} Treffer</span>}
      </div>
      <p className="muted small">
        Für Moderationsfälle: Besitz übertragen oder eine Community löschen, ohne Mitglied zu sein. Kanäle und
        Nachrichten sind hier bewusst nicht einsehbar.
      </p>
      <div className="ia-toolbar">
        <label className="ia-search">
          <Search size={16} aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setPage(0)
            }}
            placeholder="Name der Community"
            aria-label="Communities suchen"
          />
        </label>
      </div>
      {error && <p className="error">{error}</p>}
      {notice && <p className="success">{notice}</p>}
      {data && (
        <>
          <div className="ia-table-wrap">
            <table className="ia-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Besitzer</th>
                  <th className="num">Mitglieder</th>
                  <th className="num">Speicher</th>
                  <th>Erstellt</th>
                  <th aria-label="Aktionen" />
                </tr>
              </thead>
              <tbody>
                {data.items.map((c) => (
                  <tr key={c.id}>
                    <td>
                      <span className="truncate ia-strong" title={c.name}>
                        {c.name}
                      </span>
                    </td>
                    <td>
                      <span className="truncate" title={`@${c.owner.username}`}>
                        {nameOf(c.owner)}
                      </span>
                    </td>
                    <td className="num">{c.member_count}</td>
                    <td className="num">{formatBytes(c.storage_bytes)}</td>
                    <td>{fmtDate(c.created_at)}</td>
                    <td>
                      <div className="ia-row-actions">
                        <button
                          type="button"
                          className="icon-button small"
                          title="Besitz übertragen"
                          aria-label={`Besitz von ${c.name} übertragen`}
                          onClick={() => setTransferring(c)}
                        >
                          <Crown size={16} />
                        </button>
                        <button
                          type="button"
                          className="icon-button small danger"
                          title="Community löschen"
                          aria-label={`${c.name} löschen`}
                          onClick={() => setDeleting(c)}
                        >
                          <Trash2 size={16} />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {data.items.length === 0 && <p className="muted ia-empty">Keine Communities gefunden.</p>}
          </div>
          <Pager page={page} perPage={PER_PAGE} total={data.total} onPage={setPage} />
        </>
      )}

      {deleting && (
        <DeleteCommunityDialog
          community={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={() => {
            setNotice(`„${deleting.name}“ wurde gelöscht.`)
            setReload((n) => n + 1)
          }}
        />
      )}
      {transferring && (
        <TransferDialog
          community={transferring}
          onClose={() => setTransferring(null)}
          onDone={(owner) => {
            setNotice(`„${transferring.name}“ gehört jetzt ${nameOf(owner)}.`)
            setReload((n) => n + 1)
          }}
        />
      )}
    </div>
  )
}

function DeleteCommunityDialog({
  community,
  onClose,
  onDeleted,
}: {
  community: AdminCommunity
  onClose: () => void
  onDeleted: () => void
}) {
  const [confirmName, setConfirmName] = useState('')
  return (
    <ConfirmDialog
      title="Community löschen"
      confirmLabel="Endgültig löschen"
      disabled={confirmName.trim() !== community.name.trim()}
      onClose={onClose}
      onConfirm={async () => {
        await adminApi.deleteCommunity(community.id)
        onDeleted()
      }}
    >
      <p>
        <strong>{community.name}</strong> wird mit allen Kanälen, Nachrichten und Dateien gelöscht (
        {community.member_count} Mitglieder, {formatBytes(community.storage_bytes)}). Das lässt sich nicht rückgängig
        machen.
      </p>
      <label className="field">
        <span>Zur Bestätigung den Namen eingeben</span>
        <input value={confirmName} onChange={(e) => setConfirmName(e.target.value)} placeholder={community.name} />
      </label>
    </ConfirmDialog>
  )
}

function TransferDialog({
  community,
  onClose,
  onDone,
}: {
  community: AdminCommunity
  onClose: () => void
  onDone: (owner: UserPublic) => void
}) {
  const [members, setMembers] = useState<UserPublic[] | null>(null)
  const [target, setTarget] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    adminApi
      .communityMembers(community.id)
      .then((list) => setMembers(list.filter((m) => m.id !== community.owner.id)))
      .catch((e) => setError(errorText(e)))
  }, [community])

  return (
    <ConfirmDialog
      title="Besitz übertragen"
      confirmLabel="Übertragen"
      danger={false}
      disabled={!target}
      onClose={onClose}
      onConfirm={async () => {
        await adminApi.transferCommunity(community.id, target)
        const owner = members?.find((m) => m.id === target)
        if (owner) onDone(owner)
      }}
    >
      <p>
        Neuer Besitzer von <strong>{community.name}</strong> (bisher {nameOf(community.owner)}):
      </p>
      {error && <p className="error">{error}</p>}
      {members === null ? (
        !error && <p className="muted">Lade Mitglieder …</p>
      ) : members.length === 0 ? (
        <p className="muted">Es gibt keine weiteren Mitglieder.</p>
      ) : (
        <select value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Neuer Besitzer">
          <option value="">Mitglied wählen …</option>
          {members.map((m) => (
            <option key={m.id} value={m.id}>
              {nameOf(m)} (@{m.username})
            </option>
          ))}
        </select>
      )}
    </ConfirmDialog>
  )
}
