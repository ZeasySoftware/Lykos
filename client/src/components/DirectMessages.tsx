import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { DmChannel, UserPublic } from '../lib/api'
import { errorText } from '../lib/errors'
import { buildDmMenu, leaveGroupDm } from '../lib/contextMenus'
import { openDialog } from '../lib/dialogs'
import { useContextMenu } from '../lib/menuStore'
import { isChannelMuted, useMutesVersion } from '../lib/notify'
import { statusOf } from '../lib/presence'
import { isDmUnread } from '../lib/unread'
import { incomingCount } from '../lib/friends'
import { useFriends } from '../lib/friendsStore'
import { useCall, useCalls } from '../lib/callStore'
import { displayName, dmTitle, useStore } from '../store'
import { Modal } from './Modal'
import { EmojiInput } from './EmojiButton'
import { Avatar } from './Presence'
import { Contact, LogOut, Phone, PhoneIncoming, Plus, Settings, Users } from 'lucide-react'

/** Höchstzahl an Empfängern einer Gruppen-DM (ohne einen selbst), wie auf dem Server. */
const MAX_RECIPIENTS = 9

/** Neueste Aktivität zuerst (UUIDv7: letzte Nachricht bzw. Erstellung). */
function activity(dm: DmChannel): string {
  return dm.last_message_id ?? dm.id
}

/** Seitenleiste der Direktnachrichten-Ansicht. */
export function DmSidebar() {
  const dms = useStore((s) => s.dms)
  const selectedDmId = useStore((s) => s.selectedDmId)
  const selectDm = useStore((s) => s.selectDm)
  const openFriends = useStore((s) => s.openFriends)
  const me = useStore((s) => s.user)
  const requests = useFriends((s) => incomingCount(Object.values(s.relationships)))
  const calls = useCalls((s) => s.calls)
  useMutesVersion()

  // Leere 1:1-DMs (nur geöffnet, nie geschrieben) erst zeigen, wenn es Nachrichten gibt – oder einen Anruf.
  const list = Object.values(dms)
    .filter((dm) => dm.kind === 'group_dm' || dm.last_message_id || dm.id === selectedDmId || dm.id in calls)
    .sort((a, b) => (activity(a) < activity(b) ? 1 : -1))

  return (
    <aside className="sidebar">
      <header className="sidebar-header">
        <span className="truncate">Direktnachrichten</span>
        <span className="sidebar-header-actions">
          <button type="button" className="icon-button" title="Neue Nachricht" onClick={() => openDialog({ kind: 'newDm' })}>
            <Plus size={18} />
          </button>
          <button
            type="button"
            className="icon-button"
            title="Einstellungen für Direktnachrichten"
            onClick={() => openDialog({ kind: 'settings', tab: 'privacy' })}
          >
            <Settings size={18} />
          </button>
        </span>
      </header>

      <div className="channel-list">
        <button
          type="button"
          className={`channel dm-entry dm-friends-entry${selectedDmId === null ? ' active' : ''}`}
          aria-current={selectedDmId === null}
          onClick={openFriends}
        >
          <span className="channel-icon" aria-hidden="true">
            <Contact size={20} />
          </span>
          <span className="truncate">Freunde</span>
          {requests > 0 && (
            <span className="mention-badge" aria-label={`${requests} offene Freundschaftsanfragen`}>
              {requests > 99 ? '99+' : requests}
            </span>
          )}
        </button>
        {list.length === 0 && (
          <p className="sidebar-empty">Noch keine Direktnachrichten. Starte eine über das + oben.</p>
        )}
        {list.map((dm) => (
          <DmButton
            key={dm.id}
            dm={dm}
            meId={me?.id}
            active={dm.id === selectedDmId}
            onClick={() => selectDm(dm.id)}
            onSettings={dm.kind === 'group_dm' ? () => openDialog({ kind: 'groupDm', dmId: dm.id }) : undefined}
          />
        ))}
      </div>


    </aside>
  )
}

function DmButton({
  dm,
  meId,
  active,
  onClick,
  onSettings,
}: {
  dm: DmChannel
  meId: string | undefined
  active: boolean
  onClick: () => void
  onSettings?: () => void
}) {
  const title = dmTitle(dm, meId)
  const unread = !active && isDmUnread(dm)
  const other = dm.kind === 'dm' ? dm.recipients.find((u) => u.id !== meId) : undefined
  const status = useStore((s) => (other ? statusOf(s, other.id) : 'offline'))
  const muted = isChannelMuted(dm.id)
  const menu = useContextMenu((ctx) => buildDmMenu(dm.id, ctx.target), title)
  const call = useCall(dm.id)
  const inCall = (call?.voiceStates.length ?? 0) > 0
  const ringingMe = !!meId && !!call?.ringing.includes(meId)

  const button = (
    <button
      type="button"
      className={`channel dm-entry${active ? ' active' : ''}${unread && !muted ? ' unread' : ''}${muted ? ' muted-channel' : ''}`}
      aria-label={unread ? `${title} (ungelesen)` : title}
      title={title}
      onClick={onClick}
      data-menu-anchor
      {...menu}
    >
      {other ? (
        <Avatar user={other} name={displayName(other)} status={status} />
      ) : (
        <span className="avatar dm-group-avatar" aria-hidden="true">
          <Users size={16} />
        </span>
      )}
      <span className="dm-entry-text">
        <span className="truncate">{title}</span>
        {ringingMe ? (
          <span className="dm-call-hint ringing">Ruft an …</span>
        ) : inCall ? (
          <span className="dm-call-hint">
            Anruf läuft · {call!.voiceStates.length} {call!.voiceStates.length === 1 ? 'Person' : 'Personen'}
          </span>
        ) : (
          dm.kind === 'group_dm' && <span className="muted small">{dm.recipients.length} Mitglieder</span>
        )}
      </span>
      {(inCall || ringingMe) && (
        <span className={`dm-call-icon${ringingMe ? ' ringing' : ''}`} aria-hidden="true">
          {ringingMe ? <PhoneIncoming size={15} /> : <Phone size={15} />}
        </span>
      )}
      {!active && dm.mention_count > 0 && (
        <span className="mention-badge" aria-label={`${dm.mention_count} ungelesene Erwähnungen`}>
          {dm.mention_count > 99 ? '99+' : dm.mention_count}
        </span>
      )}
    </button>
  )
  if (!onSettings) return button
  return (
    <div className="channel-row">
      {button}
      <button type="button" className="channel-settings icon-button small" title="Gruppe bearbeiten" onClick={onSettings}>
        <Settings size={15} />
      </button>
    </div>
  )
}

/** „Neue Nachricht“: Personen aus gemeinsamen Communities auswählen (eine → 1:1, mehrere → Gruppe). */
export function NewDmDialog({ onClose }: { onClose: () => void }) {
  const openDm = useStore((s) => s.openDm)
  const [contacts, setContacts] = useState<UserPublic[] | null>(null)
  const [filter, setFilter] = useState('')
  const [selected, setSelected] = useState<string[]>([])
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    endpoints
      .dmContacts()
      .then(setContacts)
      .catch((e) => setError(errorText(e)))
  }, [])

  function toggle(id: string) {
    setSelected((list) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]))
  }

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (selected.length === 0) return
    setBusy(true)
    setError(null)
    try {
      await openDm(selected, selected.length > 1 ? name.trim() : undefined)
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const needle = filter.trim().toLowerCase()
  const shown = (contacts ?? []).filter(
    (u) => !needle || u.username.toLowerCase().includes(needle) || u.display_name?.toLowerCase().includes(needle),
  )
  const tooMany = selected.length > MAX_RECIPIENTS

  return (
    <Modal title="Neue Nachricht" onClose={onClose}>
      <form onSubmit={submit} className="form">
        <p className="muted small">
          Du kannst deinen Freunden schreiben und Personen, mit denen du eine Community teilst. Mit mehreren entsteht
          eine Gruppe (bis 10 Personen).
        </p>
        <label className="field">
          <span>Personen suchen</span>
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Name" autoFocus />
        </label>
        <ul className="dm-picker" aria-label="Personen">
          {contacts === null && !error && <li className="muted small">Lade …</li>}
          {contacts !== null && shown.length === 0 && <li className="muted small">Niemand gefunden.</li>}
          {shown.map((u) => (
            <li key={u.id}>
              <label className="dm-picker-item">
                <input type="checkbox" checked={selected.includes(u.id)} onChange={() => toggle(u.id)} />
                <span className="truncate">{displayName(u)}</span>
                {u.display_name && <span className="muted small truncate">@{u.username}</span>}
              </label>
            </li>
          ))}
        </ul>
        {selected.length > 1 && (
          <label className="field">
            <span>Gruppenname (optional)</span>
            <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} />
          </label>
        )}
        {tooMany && <p className="error small">Höchstens {MAX_RECIPIENTS} Personen auswählen.</p>}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy || selected.length === 0 || tooMany}>
            {selected.length > 1 ? 'Gruppe erstellen' : 'Nachricht schreiben'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Gruppen-DM umbenennen oder verlassen. */
export function GroupDmDialog({ dm, onClose }: { dm: DmChannel; onClose: () => void }) {
  const upsertDm = useStore((s) => s.upsertDm)
  const me = useStore((s) => s.user)
  const [name, setName] = useState(dm.name ?? '')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function save(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      upsertDm(await endpoints.renameDm(dm.id, name.trim() || null))
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }


  return (
    <Modal title="Gruppe bearbeiten" onClose={onClose}>
      <form onSubmit={save} className="form">
        <label className="field">
          <span>Gruppenname</span>
          <EmojiInput
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={100}
            placeholder={dmTitle({ ...dm, name: null }, me?.id)}
          />
        </label>
        <div>
          <span className="muted small">Mitglieder</span>
          <ul className="dm-members">
            {dm.recipients.map((u) => (
              <li key={u.id}>
                <Avatar user={u} name={displayName(u)} size={24} />
                <span className="truncate">{displayName(u)}</span>
                {u.id === me?.id && <span className="muted small">(du)</span>}
              </li>
            ))}
          </ul>
        </div>
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <div className="form-actions-left">
            <button type="button" className="button danger" onClick={() => leaveGroupDm(dm)}>
              <LogOut size={16} /> Gruppe verlassen
            </button>
          </div>
          <button type="button" className="button ghost" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            Speichern
          </button>
        </div>
      </form>
    </Modal>
  )
}
