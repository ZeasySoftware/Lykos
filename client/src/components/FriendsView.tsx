import { useState } from 'react'
import type { FormEvent } from 'react'
import type { Relationship } from '../lib/api'
import { errorText } from '../lib/errors'
import { blockUser, buildUserMenu, removeFriend } from '../lib/contextMenus'
import { FRIENDS_TABS, friendsInTab, incomingCount, relationshipShowsProfile } from '../lib/friends'
import type { FriendsTab } from '../lib/friends'
import { useFriends } from '../lib/friendsStore'
import { useContextMenu } from '../lib/menuStore'
import { presenceLabel, statusOf } from '../lib/presence'
import { runAction } from '../lib/toast'
import { displayName, useStore } from '../store'
import { Avatar } from './Presence'
import { Ban, Check, MessageCircle, Phone, Search, UserCheck, UserMinus, UserPlus, Users, X } from 'lucide-react'
import { callUser } from '../lib/callActions'
import { DrawerButton } from './MobileNav'

const EMPTY: Record<FriendsTab, string> = {
  online: 'Gerade ist keiner deiner Freunde online.',
  all: 'Du hast noch keine Freunde. Schick jemandem über „Freund hinzufügen“ eine Anfrage.',
  pending: 'Keine offenen Freundschaftsanfragen.',
  blocked: 'Du hast niemanden blockiert.',
}

const HEADINGS: Record<FriendsTab, string> = {
  online: 'Online',
  all: 'Alle Freunde',
  pending: 'Ausstehend',
  blocked: 'Blockiert',
}

/** Freundesliste im DM-Bereich (keine Unterhaltung ausgewählt): Tabs, Suche, Anfrage per Benutzername. */
export function FriendsView() {
  const relationships = useFriends((s) => s.relationships)
  const loaded = useFriends((s) => s.loaded)
  // Präsenz-Teil des Stores – Änderungen rendern die Liste neu (Tab „Online“).
  const presenceSource = useStore((s) => s.presences)
  const user = useStore((s) => s.user)
  const gatewayStatus = useStore((s) => s.gatewayStatus)
  const chosenStatus = useStore((s) => s.chosenStatus)
  const autoIdle = useStore((s) => s.autoIdle)
  const list = Object.values(relationships)
  const pending = incomingCount(list)
  const [tab, setTab] = useState<FriendsTab>(pending > 0 ? 'pending' : 'online')
  const [adding, setAdding] = useState(false)
  const [query, setQuery] = useState('')

  const status = (id: string) =>
    statusOf({ user, gatewayStatus, presences: presenceSource, chosenStatus, autoIdle }, id)
  const shown = friendsInTab(list, tab, query, status)

  return (
    <section className="friends-pane">
      <header className="pane-header friends-header">
        <DrawerButton />
        <span className="channel-icon" aria-hidden="true">
          <Users size={20} />
        </span>
        <strong>Freunde</strong>
        <div className="friends-tabs" role="tablist" aria-label="Freunde filtern">
          {FRIENDS_TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={!adding && tab === t.id}
              className={!adding && tab === t.id ? 'active' : ''}
              onClick={() => {
                setTab(t.id)
                setAdding(false)
              }}
            >
              {t.label}
              {t.id === 'pending' && pending > 0 && (
                <span className="mention-badge" aria-label={`${pending} offene Anfragen`}>
                  {pending > 99 ? '99+' : pending}
                </span>
              )}
            </button>
          ))}
        </div>
        <button
          type="button"
          className={`button small-button${adding ? '' : ' primary'} friends-add-toggle`}
          aria-pressed={adding}
          aria-label="Freund hinzufügen"
          onClick={() => setAdding((a) => !a)}
        >
          <UserPlus size={15} aria-hidden="true" /> <span className="friends-add-label">Freund hinzufügen</span>
        </button>
      </header>

      <div className="friends-body">
        {adding ? (
          <AddFriendForm onDone={() => setTab('pending')} />
        ) : (
          <>
            <label className="friends-search">
              <Search size={15} aria-hidden="true" />
              <input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Suchen"
                aria-label="Freunde durchsuchen"
              />
            </label>
            <h3 className="friends-count">
              {HEADINGS[tab]} – {shown.length}
            </h3>
            {!loaded ? (
              <p className="muted small">Lade …</p>
            ) : shown.length === 0 ? (
              <p className="friends-empty muted">{query.trim() ? 'Niemand gefunden.' : EMPTY[tab]}</p>
            ) : (
              <ul className="friends-list">
                {shown.map((r) => (
                  <FriendRow key={r.id} rel={r} status={status(r.id)} />
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </section>
  )
}

/** Freundschaftsanfrage per Benutzername. */
function AddFriendForm({ onDone }: { onDone: () => void }) {
  const requestByName = useFriends((s) => s.requestByName)
  const [username, setUsername] = useState('')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null)

  async function submit(e: FormEvent) {
    e.preventDefault()
    const name = username.trim().replace(/^@/, '')
    if (!name) return
    setBusy(true)
    setResult(null)
    try {
      const rel = await requestByName(name)
      const who = displayName(rel.user)
      setResult({
        ok: true,
        text: rel.type === 'friend' ? `Ihr seid jetzt befreundet – ${who} hatte dich schon angefragt.` : `Anfrage an ${who} gesendet.`,
      })
      setUsername('')
    } catch (err) {
      setResult({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="friends-add" onSubmit={submit}>
      <h2>Freund hinzufügen</h2>
      <p className="muted small">
        Gib den Benutzernamen ein (Groß-/Kleinschreibung egal). Wer Anfragen annimmt, legt jeder selbst fest.
      </p>
      <div className={`friends-add-row${result ? (result.ok ? ' ok' : ' failed') : ''}`}>
        <input
          value={username}
          onChange={(e) => {
            setUsername(e.target.value)
            setResult(null)
          }}
          placeholder="Benutzername"
          aria-label="Benutzername"
          maxLength={64}
          autoFocus
        />
        <button type="submit" className="button primary small-button" disabled={busy || !username.trim()}>
          Anfrage senden
        </button>
      </div>
      {result && (
        <p className={result.ok ? 'friends-add-ok small' : 'error small'} role="status">
          {result.text}{' '}
          {result.ok && (
            <button type="button" className="link-button" onClick={onDone}>
              Ausstehende ansehen
            </button>
          )}
        </p>
      )}
    </form>
  )
}

function FriendRow({ rel, status }: { rel: Relationship; status: ReturnType<typeof statusOf> }) {
  const openDm = useStore((s) => s.openDm)
  const openProfile = useStore((s) => s.openProfile)
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const addFriend = useFriends((s) => s.addFriend)
  const remove = useFriends((s) => s.remove)
  const name = displayName(rel.user)
  const menu = useContextMenu((ctx) => buildUserMenu({ userId: rel.id, communityId: null, target: ctx.target }), name)

  const subline =
    rel.type === 'friend'
      ? presenceLabel[status]
      : rel.type === 'pending_in'
        ? 'Eingehende Freundschaftsanfrage'
        : rel.type === 'pending_out'
          ? 'Ausgehende Freundschaftsanfrage'
          : 'Blockiert'

  const message = () => runAction(() => openDm([rel.id]))
  // Eigene Anfrage/Blockade gibt keine Profilsicht (Server antwortet 404) – dann nur Name und Avatar.
  const canOpenProfile = relationshipShowsProfile(rel.type)

  return (
    <li
      className="friend-row"
      data-menu-anchor
      {...menu}
      onDoubleClick={rel.type === 'friend' ? message : undefined}
    >
      <button
        type="button"
        className="friend-who"
        title={canOpenProfile ? 'Profil anzeigen' : undefined}
        onClick={
          canOpenProfile ? (e) => openProfile(rel.id, null, e.currentTarget.getBoundingClientRect()) : undefined
        }
      >
        <Avatar user={rel.user} name={name} status={rel.type === 'friend' ? status : undefined} size={36} />
        <span className="friend-names">
          <span className="truncate">
            <strong>{name}</strong>
            {rel.user.display_name && <span className="muted small"> @{rel.user.username}</span>}
          </span>
          <span className="muted small truncate">{subline}</span>
        </span>
      </button>
      <span className="friend-actions">
        {rel.type === 'friend' && (
          <>
            <button type="button" className="icon-button" title="Nachricht senden" onClick={message}>
              <MessageCircle size={18} />
            </button>
            {voiceEnabled && (
              <button
                type="button"
                className="icon-button"
                title="Anrufen"
                aria-label={`${name} anrufen`}
                onClick={() => runAction(() => callUser(rel.id))}
              >
                <Phone size={18} />
              </button>
            )}
            <button
              type="button"
              className="icon-button danger-hover"
              title="Freund entfernen"
              onClick={() => removeFriend(rel.id, name)}
            >
              <UserMinus size={18} />
            </button>
          </>
        )}
        {rel.type === 'pending_in' && (
          <>
            <button
              type="button"
              className="icon-button ok-hover"
              title="Annehmen"
              onClick={() => runAction(() => addFriend(rel.id))}
            >
              <Check size={18} />
            </button>
            <button
              type="button"
              className="icon-button danger-hover"
              title="Ablehnen"
              onClick={() => runAction(() => remove(rel.id))}
            >
              <X size={18} />
            </button>
          </>
        )}
        {rel.type === 'pending_out' && (
          <button
            type="button"
            className="icon-button danger-hover"
            title="Anfrage zurückziehen"
            onClick={() => runAction(() => remove(rel.id))}
          >
            <X size={18} />
          </button>
        )}
        {rel.type === 'blocked' ? (
          <button
            type="button"
            className="icon-button"
            title="Blockade aufheben"
            onClick={() => runAction(() => remove(rel.id))}
          >
            <UserCheck size={18} />
          </button>
        ) : (
          rel.type !== 'friend' && (
            <button type="button" className="icon-button danger-hover" title="Blockieren" onClick={() => blockUser(rel.id, name)}>
              <Ban size={18} />
            </button>
          )
        )}
      </span>
    </li>
  )
}
