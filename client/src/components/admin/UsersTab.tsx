import { useCallback, useEffect, useState } from 'react'
import { ArrowLeft, Ban, KeyRound, LogOut, Search, ShieldCheck, ShieldOff, Trash2, Unlock } from 'lucide-react'
import { adminApi } from '../../lib/adminApi'
import type { AdminUser, AdminUserDetail, OwnedCommunities, Page, UserFilter } from '../../lib/adminApi'
import { errorText } from '../../lib/errors'
import { formatBytes } from '../../lib/uploads'
import { useStore } from '../../store'
import { Avatar } from '../Presence'
import { ConfirmDialog, CopyButton, Pager, Tag } from './common'
import { fmtDate, fmtDateTime, fmtDay, fmtNumber, nameOf } from './format'

const PER_PAGE = 25

export function UsersTab() {
  const [selected, setSelected] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<UserFilter>('all')
  const [page, setPage] = useState(0)
  const [data, setData] = useState<Page<AdminUser> | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)

  useEffect(() => {
    let cancelled = false
    // Kurz warten, damit nicht jeder Tastendruck eine Anfrage auslöst.
    const timer = setTimeout(() => {
      adminApi
        .users({ q: query.trim() || undefined, page, per_page: PER_PAGE, filter })
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
  }, [query, filter, page, reload])

  if (selected) {
    return (
      <UserDetailView
        userId={selected}
        onBack={() => {
          setSelected(null)
          setReload((n) => n + 1)
        }}
      />
    )
  }

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Nutzer</h3>
        {data && <span className="muted small">{fmtNumber(data.total)} Treffer</span>}
      </div>
      <div className="ia-toolbar">
        <label className="ia-search">
          <Search size={16} aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setPage(0)
            }}
            placeholder="Benutzername oder Anzeigename"
            aria-label="Nutzer suchen"
          />
        </label>
        <select
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value as UserFilter)
            setPage(0)
          }}
          aria-label="Filter"
        >
          <option value="all">Alle</option>
          <option value="admins">Admins</option>
          <option value="disabled">Gesperrt</option>
        </select>
      </div>
      {error && <p className="error">{error}</p>}
      {data && (
        <>
          <div className="ia-table-wrap">
            <table className="ia-table">
              <thead>
                <tr>
                  <th>Nutzer</th>
                  <th>Registriert</th>
                  <th>Zuletzt aktiv</th>
                  <th className="num">Communities</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {data.items.map((u) => (
                  <tr key={u.id} className="ia-clickable" onClick={() => setSelected(u.id)}>
                    <td>
                      <button type="button" className="ia-user-cell" onClick={() => setSelected(u.id)}>
                        <Avatar user={u} name={nameOf(u)} size={28} />
                        <span className="ia-user-names">
                          <span className="truncate">{nameOf(u)}</span>
                          <span className="muted small truncate">@{u.username}</span>
                        </span>
                      </button>
                    </td>
                    <td>{fmtDate(u.created_at)}</td>
                    <td>{fmtDay(u.last_active_on)}</td>
                    <td className="num">{u.community_count}</td>
                    <td>
                      <UserTags user={u} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {data.items.length === 0 && <p className="muted ia-empty">Keine Nutzer gefunden.</p>}
          </div>
          <Pager page={page} perPage={PER_PAGE} total={data.total} onPage={setPage} />
        </>
      )}
    </div>
  )
}

function UserTags({ user }: { user: AdminUser }) {
  return (
    <span className="ia-tags">
      {user.is_instance_admin && <Tag tone="accent">Admin</Tag>}
      {user.disabled_at && <Tag tone="danger">Gesperrt</Tag>}
      {!user.is_instance_admin && !user.disabled_at && <span className="muted small">–</span>}
    </span>
  )
}

type Dialog = 'disable' | 'sessions' | 'revoke-admin' | 'grant-admin' | 'delete' | null

function UserDetailView({ userId, onBack }: { userId: string; onBack: () => void }) {
  const me = useStore((s) => s.user)
  const [user, setUser] = useState<AdminUserDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [dialog, setDialog] = useState<Dialog>(null)
  const [resetLink, setResetLink] = useState<{ url: string; expires_at: string } | null>(null)

  const load = useCallback(() => {
    adminApi
      .user(userId)
      .then((u) => {
        setUser(u)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [userId])

  useEffect(load, [load])

  async function run(action: () => Promise<void>, done: string) {
    setNotice(null)
    setError(null)
    try {
      await action()
      setNotice(done)
      load()
    } catch (e) {
      setError(errorText(e))
    }
  }

  const isSelf = me?.id === userId
  const owned = user?.communities.filter((c) => c.is_owner) ?? []

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <button type="button" className="button small-button ghost" onClick={onBack}>
          <ArrowLeft size={14} aria-hidden="true" />
          Alle Nutzer
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {notice && <p className="success">{notice}</p>}
      {user && (
        <>
          <div className="ia-card ia-user-head">
            <Avatar user={user} name={nameOf(user)} size={56} />
            <div className="ia-user-names">
              <h3 className="truncate">{nameOf(user)}</h3>
              <span className="muted truncate">@{user.username}</span>
              <UserTags user={user} />
            </div>
          </div>

          <dl className="ia-facts">
            <div>
              <dt>Registriert</dt>
              <dd>{fmtDate(user.created_at)}</dd>
            </div>
            <div>
              <dt>Zuletzt aktiv</dt>
              <dd>{fmtDay(user.last_active_on)}</dd>
            </div>
            <div>
              <dt>Aktive Sitzungen</dt>
              <dd>{user.session_count}</dd>
            </div>
            <div>
              <dt>Nachrichten</dt>
              <dd>{fmtNumber(user.message_count)}</dd>
            </div>
            <div>
              <dt>Speicher</dt>
              <dd>{formatBytes(user.storage_bytes)}</dd>
            </div>
            <div>
              <dt>Registriert mit</dt>
              <dd>
                {user.registered_with
                  ? `Einladung ${user.registered_with.code}${user.registered_with.note ? ` (${user.registered_with.note})` : ''}`
                  : '–'}
              </dd>
            </div>
            {user.disabled_at && (
              <div>
                <dt>Gesperrt seit</dt>
                <dd>{fmtDateTime(user.disabled_at)}</dd>
              </div>
            )}
          </dl>

          <h4 className="ia-subhead">Communities ({user.communities.length})</h4>
          {user.communities.length === 0 ? (
            <p className="muted small">Keine.</p>
          ) : (
            <ul className="ia-chips">
              {user.communities.map((c) => (
                <li key={c.id} title={`Beigetreten ${fmtDate(c.joined_at)}`}>
                  {c.name}
                  {c.is_owner && <Tag tone="accent">Besitzer</Tag>}
                </li>
              ))}
            </ul>
          )}

          <h4 className="ia-subhead">Aktionen</h4>
          <div className="ia-actions">
            {user.disabled_at ? (
              <button
                type="button"
                className="button"
                onClick={() => void run(() => adminApi.enableUser(userId), 'Sperre aufgehoben.')}
              >
                <Unlock size={16} aria-hidden="true" />
                Entsperren
              </button>
            ) : (
              <button type="button" className="button danger" disabled={isSelf} onClick={() => setDialog('disable')}>
                <Ban size={16} aria-hidden="true" />
                Sperren
              </button>
            )}
            <button type="button" className="button" onClick={() => setDialog('sessions')}>
              <LogOut size={16} aria-hidden="true" />
              Sitzungen beenden
            </button>
            {user.is_instance_admin ? (
              <button type="button" className="button" onClick={() => setDialog('revoke-admin')}>
                <ShieldOff size={16} aria-hidden="true" />
                Admin entziehen
              </button>
            ) : (
              <button type="button" className="button" onClick={() => setDialog('grant-admin')}>
                <ShieldCheck size={16} aria-hidden="true" />
                Zum Admin machen
              </button>
            )}
            <button
              type="button"
              className="button"
              onClick={() =>
                void run(async () => {
                  setResetLink(await adminApi.passwordResetLink(userId))
                }, 'Reset-Link erzeugt.')
              }
            >
              <KeyRound size={16} aria-hidden="true" />
              Passwort-Reset-Link
            </button>
            <button type="button" className="button danger" disabled={isSelf} onClick={() => setDialog('delete')}>
              <Trash2 size={16} aria-hidden="true" />
              Konto löschen
            </button>
          </div>
          {isSelf && <p className="muted small">Das eigene Konto kannst du hier weder sperren noch löschen.</p>}

          {resetLink && (
            <div className="ia-card ia-created">
              <span className="small">
                Einmaliger Link zum Zurücksetzen des Passworts – gültig bis {fmtDateTime(resetLink.expires_at)}. Schick
                ihn {nameOf(user)} auf einem sicheren Weg; er wird nur jetzt angezeigt. Ein neuer Link macht diesen
                ungültig.
              </span>
              <div className="ia-link-row">
                <input readOnly value={resetLink.url} onFocus={(e) => e.target.select()} aria-label="Reset-Link" />
                <CopyButton text={resetLink.url} />
              </div>
            </div>
          )}

          {dialog === 'disable' && (
            <ConfirmDialog
              title="Konto sperren"
              confirmLabel="Sperren"
              onClose={() => setDialog(null)}
              onConfirm={async () => {
                await adminApi.disableUser(userId)
                setNotice('Konto gesperrt.')
                load()
              }}
            >
              <p>
                <strong>{nameOf(user)}</strong> wird sofort abgemeldet, aus Sprachkanälen geworfen und kann sich nicht
                mehr anmelden, bis du die Sperre aufhebst. Daten bleiben erhalten.
              </p>
            </ConfirmDialog>
          )}
          {dialog === 'sessions' && (
            <ConfirmDialog
              title="Sitzungen beenden"
              confirmLabel="Alle beenden"
              onClose={() => setDialog(null)}
              onConfirm={async () => {
                await adminApi.revokeUserSessions(userId)
                setNotice('Sitzungen beendet.')
                load()
              }}
            >
              <p>
                Alle Geräte von <strong>{nameOf(user)}</strong> werden abgemeldet
                {isSelf ? ' (außer diesem)' : ''}. Eine neue Anmeldung bleibt möglich.
              </p>
            </ConfirmDialog>
          )}
          {dialog === 'grant-admin' && (
            <ConfirmDialog
              title="Zum Instanz-Admin machen"
              confirmLabel="Admin machen"
              danger={false}
              onClose={() => setDialog(null)}
              onConfirm={async () => {
                await adminApi.grantAdmin(userId)
                setNotice('Admin-Recht vergeben.')
                load()
              }}
            >
              <p>
                <strong>{nameOf(user)}</strong> kann danach die gesamte Instanz verwalten: Nutzer sperren und löschen,
                Communities löschen, Einstellungen ändern.
              </p>
            </ConfirmDialog>
          )}
          {dialog === 'revoke-admin' && (
            <ConfirmDialog
              title="Admin-Recht entziehen"
              confirmLabel="Entziehen"
              onClose={() => setDialog(null)}
              onConfirm={async () => {
                await adminApi.revokeAdmin(userId)
                setNotice('Admin-Recht entzogen.')
                load()
              }}
            >
              <p>
                {isSelf ? (
                  <>Du verlierst damit selbst den Zugang zur Instanz-Verwaltung.</>
                ) : (
                  <>
                    <strong>{nameOf(user)}</strong> verliert den Zugang zur Instanz-Verwaltung.
                  </>
                )}
              </p>
            </ConfirmDialog>
          )}
          {dialog === 'delete' && (
            <DeleteUserDialog
              user={user}
              ownedCount={owned.length}
              onClose={() => setDialog(null)}
              onDeleted={onBack}
            />
          )}
        </>
      )}
    </div>
  )
}

function DeleteUserDialog({
  user,
  ownedCount,
  onClose,
  onDeleted,
}: {
  user: AdminUserDetail
  ownedCount: number
  onClose: () => void
  onDeleted: () => void
}) {
  const [deleteMessages, setDeleteMessages] = useState(false)
  const [owned, setOwned] = useState<OwnedCommunities>('transfer')
  const [confirmName, setConfirmName] = useState('')

  return (
    <ConfirmDialog
      title="Konto löschen"
      confirmLabel="Endgültig löschen"
      disabled={confirmName.trim() !== user.username}
      onClose={onClose}
      onConfirm={async () => {
        await adminApi.deleteUser(user.id, {
          delete_messages: deleteMessages,
          owned_communities: ownedCount > 0 ? owned : undefined,
        })
        onDeleted()
      }}
    >
      <p>
        Das Konto <strong>@{user.username}</strong> wird endgültig gelöscht. Das lässt sich nicht rückgängig machen.
      </p>
      <label className="checkbox">
        <input type="checkbox" checked={deleteMessages} onChange={(e) => setDeleteMessages(e.target.checked)} />
        Auch alle Nachrichten löschen ({fmtNumber(user.message_count)}) – sonst bleiben sie als „Gelöschter Nutzer“
      </label>
      {ownedCount > 0 && (
        <fieldset className="radio-group ia-radio-stack">
          <legend>
            {ownedCount === 1 ? 'Eine eigene Community' : `${ownedCount} eigene Communities`} – was passiert damit?
          </legend>
          <label>
            <input type="radio" checked={owned === 'transfer'} onChange={() => setOwned('transfer')} />
            An das am längsten beigetretene Mitglied übertragen (ohne weitere Mitglieder: löschen)
          </label>
          <label>
            <input type="radio" checked={owned === 'delete'} onChange={() => setOwned('delete')} />
            Samt Inhalt löschen
          </label>
        </fieldset>
      )}
      <label className="field">
        <span>Zur Bestätigung den Benutzernamen eingeben</span>
        <input value={confirmName} onChange={(e) => setConfirmName(e.target.value)} placeholder={user.username} />
      </label>
    </ConfirmDialog>
  )
}
