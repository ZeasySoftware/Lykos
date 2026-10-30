import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { DmChannel, Message, UserPublic } from '../lib/api'
import { errorText } from '../lib/errors'
import { buildDmMenu, leaveGroupDm } from '../lib/contextMenus'
import { runAction } from '../lib/toast'
import { openDialog } from '../lib/dialogs'
import { addableContacts, dmSections, remainingSlots } from '../lib/dmList'
import { useContextMenu } from '../lib/menuStore'
import { isChannelMuted, useMutesVersion } from '../lib/notify'
import { statusOf } from '../lib/presence'
import { isDmUnread } from '../lib/unread'
import { incomingCount } from '../lib/friends'
import { useFriends } from '../lib/friendsStore'
import { useCall, useCalls } from '../lib/callStore'
import { displayName, dmTitle, recipientAddText, useStore } from '../store'
import { Modal } from './Modal'
import { EmojiInput } from './EmojiButton'
import { Avatar } from './Presence'
import {
  Contact,
  LogOut,
  Phone,
  PhoneIncoming,
  Pin,
  PinOff,
  Plus,
  Settings,
  UserPlus,
  Users,
  UsersRound,
} from 'lucide-react'

/** Höchstzahl an Teilnehmern einer Gruppen-DM (inkl. einem selbst), wie auf dem Server. */
const MAX_GROUP_SIZE = 10
/** Höchstzahl an Empfängern einer neuen DM (ohne einen selbst). */
const MAX_RECIPIENTS = MAX_GROUP_SIZE - 1

const timeFmt = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' })

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

  // Angepinnte oben (auch leere 1:1-DMs), darunter nach Aktivität; leere 1:1-DMs erst mit Nachrichten oder Anruf.
  const { pinned, recent } = dmSections(Object.values(dms), { selectedId: selectedDmId, callIds: calls })

  const entry = (dm: DmChannel) => (
    <DmButton
      key={dm.id}
      dm={dm}
      meId={me?.id}
      active={dm.id === selectedDmId}
      onClick={() => selectDm(dm.id)}
      onSettings={dm.kind === 'group_dm' ? () => openDialog({ kind: 'groupDm', dmId: dm.id }) : undefined}
    />
  )

  return (
    <aside className="sidebar">
      <header className="sidebar-header">
        <span className="truncate" title="Direktnachrichten">Nachrichten</span>
        <span className="sidebar-header-actions">
          <button type="button" className="icon-button" title="Neue Nachricht" onClick={() => openDialog({ kind: 'newDm' })}>
            <Plus size={18} />
          </button>
          <button
            type="button"
            className="icon-button"
            title="Gruppe erstellen"
            onClick={() => openDialog({ kind: 'createGroupDm' })}
          >
            <UsersRound size={18} />
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
        {pinned.length === 0 && recent.length === 0 && (
          <p className="sidebar-empty">Noch keine Direktnachrichten. Starte eine über das + oben.</p>
        )}
        {pinned.length > 0 ? (
          <>
            <section className="channel-group dm-group" aria-label="Angepinnt">
              <header>
                <span>Angepinnt</span>
              </header>
              {pinned.map(entry)}
            </section>
            {recent.length > 0 && (
              <section className="channel-group dm-group" aria-label="Direktnachrichten">
                <header>
                  <span>Direktnachrichten</span>
                </header>
                {recent.map(entry)}
              </section>
            )}
          </>
        ) : (
          recent.map(entry)
        )}
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
  const setDmPinned = useStore((s) => s.setDmPinned)
  const title = dmTitle(dm, meId)
  const unread = !active && isDmUnread(dm)
  const other = dm.kind === 'dm' ? dm.recipients.find((u) => u.id !== meId) : undefined
  const status = useStore((s) => (other ? statusOf(s, other.id) : 'offline'))
  const muted = isChannelMuted(dm.id)
  const menu = useContextMenu((ctx) => buildDmMenu(dm.id, ctx.target), title)
  const call = useCall(dm.id)
  const inCall = (call?.voiceStates.length ?? 0) > 0
  const ringingMe = !!meId && !!call?.ringing.includes(meId)
  const pinned = !!dm.pinned_at

  return (
    <div className={`channel-row dm-row${onSettings ? ' has-settings' : ''}${pinned ? ' pinned' : ''}`}>
      <button
        type="button"
        className={`channel dm-entry${active ? ' active' : ''}${unread && !muted ? ' unread' : ''}${muted ? ' muted-channel' : ''}`}
        aria-label={`${title}${pinned ? ' (angepinnt)' : ''}${unread ? ' (ungelesen)' : ''}`}
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
          <span className="dm-entry-title">
            <span className="truncate">{title}</span>
            {pinned && <Pin size={12} className="dm-pin-mark" aria-hidden="true" />}
          </span>
          {ringingMe ? (
            <span className="dm-call-hint ringing">Ruft an …</span>
          ) : inCall ? (
            <span className="dm-call-hint">
              Anruf läuft · {call!.voiceStates.length} {call!.voiceStates.length === 1 ? 'Person' : 'Personen'}
            </span>
          ) : (
            dm.kind === 'group_dm' && (
              <span className="muted small">
                {dm.recipients.length} {dm.recipients.length === 1 ? 'Mitglied' : 'Mitglieder'}
              </span>
            )
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
      <span className="dm-entry-actions">
        <button
          type="button"
          className="icon-button small"
          title={pinned ? 'Lösen' : 'Anpinnen'}
          aria-label={`${title} ${pinned ? 'lösen' : 'anpinnen'}`}
          aria-pressed={pinned}
          onClick={() => runAction(() => setDmPinned(dm.id, !pinned))}
        >
          {pinned ? <PinOff size={15} /> : <Pin size={15} />}
        </button>
        {onSettings && (
          <button type="button" className="icon-button small" title="Gruppe bearbeiten" onClick={onSettings}>
            <Settings size={15} />
          </button>
        )}
      </span>
    </div>
  )
}

/** Kontakte für die Personenauswahl laden (Freunde + Mitglieder gemeinsamer Communities). */
function useDmContacts(): [UserPublic[] | null, string | null] {
  const [contacts, setContacts] = useState<UserPublic[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    endpoints
      .dmContacts()
      .then(setContacts)
      .catch((e) => setError(errorText(e)))
  }, [])
  return [contacts, error]
}

/** Personenauswahl mit Suche (für neue DMs, Gruppen und „Personen hinzufügen“). */
function PersonPicker({
  contacts,
  selected,
  onToggle,
  autoFocus = true,
}: {
  contacts: UserPublic[] | null
  selected: string[]
  onToggle: (id: string) => void
  autoFocus?: boolean
}) {
  const [filter, setFilter] = useState('')
  const needle = filter.trim().toLowerCase()
  const shown = (contacts ?? []).filter(
    (u) => !needle || u.username.toLowerCase().includes(needle) || u.display_name?.toLowerCase().includes(needle),
  )
  return (
    <>
      <label className="field">
        <span>Personen suchen</span>
        <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Name" autoFocus={autoFocus} />
      </label>
      <ul className="dm-picker" aria-label="Personen">
        {contacts === null && <li className="muted small">Lade …</li>}
        {contacts !== null && shown.length === 0 && <li className="muted small">Niemand gefunden.</li>}
        {shown.map((u) => (
          <li key={u.id}>
            <label className="dm-picker-item">
              <input type="checkbox" checked={selected.includes(u.id)} onChange={() => onToggle(u.id)} />
              <Avatar user={u} name={displayName(u)} size={24} />
              <span className="truncate">{displayName(u)}</span>
              {u.display_name && <span className="muted small truncate">@{u.username}</span>}
            </label>
          </li>
        ))}
      </ul>
    </>
  )
}

function useSelection(): [string[], (id: string) => void] {
  const [selected, setSelected] = useState<string[]>([])
  const toggle = (id: string) => setSelected((list) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]))
  return [selected, toggle]
}

/** „Neue Nachricht“: Personen aus gemeinsamen Communities auswählen (eine → 1:1, mehrere → Gruppe). */
export function NewDmDialog({ onClose }: { onClose: () => void }) {
  const openDm = useStore((s) => s.openDm)
  const [contacts, loadError] = useDmContacts()
  const [selected, toggle] = useSelection()
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

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

  const tooMany = selected.length > MAX_RECIPIENTS

  return (
    <Modal title="Neue Nachricht" onClose={onClose}>
      <form onSubmit={submit} className="form">
        <p className="muted small">
          Du kannst deinen Freunden schreiben und Personen, mit denen du eine Community teilst. Mit mehreren entsteht
          eine Gruppe (bis 10 Personen).
        </p>
        <PersonPicker contacts={loadError ? [] : contacts} selected={selected} onToggle={toggle} />
        {selected.length > 1 && (
          <label className="field">
            <span>Gruppenname (optional)</span>
            <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} />
          </label>
        )}
        {tooMany && <p className="error small">Höchstens {MAX_RECIPIENTS} Personen auswählen.</p>}
        {(error ?? loadError) && <p className="error">{error ?? loadError}</p>}
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

/** „Gruppe erstellen“: Name und Personen – auch mit nur einer weiteren Person oder ganz allein. */
export function CreateGroupDialog({ onClose }: { onClose: () => void }) {
  const createGroupDm = useStore((s) => s.createGroupDm)
  const [contacts, loadError] = useDmContacts()
  const [selected, toggle] = useSelection()
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await createGroupDm(selected, name.trim())
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const tooMany = selected.length > MAX_RECIPIENTS

  return (
    <Modal title="Gruppe erstellen" onClose={onClose}>
      <form onSubmit={submit} className="form">
        <p className="muted small">
          Ein Gruppenchat für bis zu 10 Personen, unabhängig von Communities. Du kannst auch erst einmal allein starten
          und später Leute hinzufügen.
        </p>
        <label className="field">
          <span>Gruppenname (optional)</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} autoFocus />
        </label>
        <PersonPicker contacts={loadError ? [] : contacts} selected={selected} onToggle={toggle} autoFocus={false} />
        {tooMany && <p className="error small">Höchstens {MAX_RECIPIENTS} Personen auswählen.</p>}
        {(error ?? loadError) && <p className="error">{error ?? loadError}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy || tooMany}>
            Gruppe erstellen
          </button>
        </div>
      </form>
    </Modal>
  )
}

/**
 * „Personen hinzufügen“: In einer Gruppe kommen die Ausgewählten dazu; in einer 1:1-DM entsteht eine neue Gruppe
 * aus beiden und den Ausgewählten (die Unterhaltung zu zweit bleibt).
 */
export function AddToDmDialog({ dm, onClose }: { dm: DmChannel; onClose: () => void }) {
  const addToDm = useStore((s) => s.addToDm)
  const me = useStore((s) => s.user)
  const [contacts, loadError] = useDmContacts()
  const [selected, toggle] = useSelection()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const group = dm.kind === 'group_dm'
  const free = remainingSlots(dm.recipients.length, MAX_GROUP_SIZE)
  const partner = group ? undefined : dm.recipients.find((u) => u.id !== me?.id)

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (selected.length === 0) return
    setBusy(true)
    setError(null)
    try {
      await addToDm(dm.id, selected)
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const tooMany = selected.length > free
  const shown = contacts && addableContacts(contacts, dm.recipients)

  return (
    <Modal title={group ? `Personen zu ${dmTitle(dm, me?.id)} hinzufügen` : 'Personen hinzufügen'} onClose={onClose}>
      <form onSubmit={submit} className="form">
        <p className="muted small">
          {group
            ? free > 0
              ? `Noch ${free} ${free === 1 ? 'Platz' : 'Plätze'} frei. Neue Mitglieder sehen den bisherigen Verlauf.`
              : 'Die Gruppe ist voll (10 Personen).'
            : `Es entsteht eine neue Gruppe mit ${partner ? displayName(partner) : 'euch'} und den Ausgewählten. Eure Unterhaltung zu zweit bleibt bestehen.`}
        </p>
        <PersonPicker contacts={loadError ? [] : shown} selected={selected} onToggle={toggle} />
        {tooMany && (
          <p className="error small">
            Höchstens {free} {free === 1 ? 'Person' : 'Personen'} auswählen.
          </p>
        )}
        {(error ?? loadError) && <p className="error">{error ?? loadError}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy || selected.length === 0 || tooMany}>
            {group ? 'Hinzufügen' : 'Gruppe erstellen'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Knopf „Personen hinzufügen“ im Kopf einer DM. */
export function DmAddPeopleButton({ channelId }: { channelId: string }) {
  const dm = useStore((s) => s.dms[channelId])
  const me = useStore((s) => s.user?.id)
  const other = dm?.kind === 'dm' ? dm.recipients.find((u) => u.id !== me) : undefined
  const blocked = useFriends((s) => !!other && s.relationships[other.id]?.type === 'blocked')
  if (!dm || blocked || (dm.kind === 'dm' && !other)) return null
  const full = dm.kind === 'group_dm' && remainingSlots(dm.recipients.length, MAX_GROUP_SIZE) === 0
  const title = full ? 'Die Gruppe ist voll (10 Personen)' : 'Personen hinzufügen'
  return (
    <button
      type="button"
      className="icon-button"
      title={title}
      aria-label={title}
      disabled={full}
      onClick={() => openDialog({ kind: 'addToDm', dmId: channelId })}
    >
      <UserPlus size={18} />
    </button>
  )
}

/** Systemnachricht `recipient_add`: „X hat Y zur Gruppe hinzugefügt.“ */
export function RecipientAddMessage({ message }: { message: Message }) {
  const me = useStore((s) => s.user?.id)
  const dm = useStore((s) => s.dms[message.channel_id])
  const openProfile = useStore((s) => s.openProfile)
  const author = message.author
  return (
    <div className="message system-message recipient-add" data-message-id={message.id}>
      <UserPlus size={16} className="system-message-icon" aria-hidden="true" />
      <span className="system-message-text">
        {author ? (
          <button
            type="button"
            className="message-author"
            onClick={(e) => openProfile(author.id, null, e.currentTarget.getBoundingClientRect())}
          >
            {author.id === me ? 'Du' : displayName(author)}
          </button>
        ) : (
          'Jemand'
        )}{' '}
        {author?.id === me ? recipientAddText(message, dm, me).replace(/^hat /, 'hast ') : recipientAddText(message, dm, me)}
      </span>
      <time className="message-time" dateTime={message.created_at}>
        {timeFmt.format(new Date(message.created_at))}
      </time>
    </div>
  )
}

/** Gruppen-DM umbenennen, Personen hinzufügen oder verlassen. */
export function GroupDmDialog({ dm, onClose }: { dm: DmChannel; onClose: () => void }) {
  const upsertDm = useStore((s) => s.upsertDm)
  const me = useStore((s) => s.user)
  const [name, setName] = useState(dm.name ?? '')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const full = remainingSlots(dm.recipients.length, MAX_GROUP_SIZE) === 0

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
          <div className="dm-members-header">
            <span className="muted small">Mitglieder ({dm.recipients.length}/10)</span>
            <button
              type="button"
              className="button ghost small-button"
              disabled={full}
              title={full ? 'Die Gruppe ist voll (10 Personen)' : undefined}
              onClick={() => openDialog({ kind: 'addToDm', dmId: dm.id })}
            >
              <UserPlus size={15} /> Personen hinzufügen
            </button>
          </div>
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
