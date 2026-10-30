import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { Role } from '../lib/api'
import { errorText } from '../lib/errors'
import { Permission, has } from '../lib/permissions'
import { sortRoles } from '../lib/roles'
import { roleColor } from '../lib/permissionLabels'
import { statusOf } from '../lib/presence'
import { displayName, useMember, useStore } from '../store'
import { Avatar } from './Presence'
import { EmojiInput } from './EmojiButton'
import { Check, Crown, Maximize2, MessageCircle, Pencil, UserCheck, UserPlus, X } from 'lucide-react'
import { useFriends, useRelationshipType } from '../lib/friendsStore'
import { friendActions } from '../lib/friends'
import { runAction } from '../lib/toast'
import { accentStyle } from '../lib/customize'
import { BannerFill } from './customize/parts'
import { ProfileBio } from './customize/ProfileView'
import { useProfileModal } from './customize/profileModal'

const CARD_WIDTH = 300
const MARGIN = 8
const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })

/**
 * Profilkarte (Popover) zum Nutzer aus `profileCard` im Store. Öffnen per `openProfile(...)` beim Klick
 * auf Name oder Avatar; schließt mit Esc oder Klick daneben.
 */
export function ProfileCard() {
  const card = useStore((s) => s.profileCard)
  if (!card) return null
  // key: Zustand (Nickname-Formular, Position) pro geöffnetem Nutzer frisch.
  return <ProfileCardInner key={`${card.userId}:${card.communityId}:${card.anchor.top}`} />
}

function ProfileCardInner() {
  const card = useStore((s) => s.profileCard)!
  const close = useStore((s) => s.closeProfile)
  const loadProfile = useStore((s) => s.loadProfile)
  const profile = useStore((s) => s.profiles[card.userId])
  const me = useStore((s) => s.user)
  const member = useMember(card.communityId, card.userId)
  const detail = useStore((s) => (card.communityId ? s.details[card.communityId] : undefined))
  const status = useStore((s) => statusOf(s, card.userId))
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [editing, setEditing] = useState(false)
  const [nickname, setNickname] = useState('')

  useEffect(() => {
    loadProfile(card.userId).catch((e) => setError(errorText(e)))
  }, [card.userId, loadProfile])

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close()
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close()
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [close])

  // Neben dem Auslöser platzieren (rechts, sonst links) und im Fenster halten.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { anchor } = card
    const height = el.offsetHeight
    let left = anchor.right + MARGIN
    if (left + CARD_WIDTH > window.innerWidth - MARGIN) left = anchor.left - CARD_WIDTH - MARGIN
    left = Math.max(MARGIN, Math.min(left, window.innerWidth - CARD_WIDTH - MARGIN))
    const top = Math.max(MARGIN, Math.min(anchor.top, window.innerHeight - height - MARGIN))
    setPos((p) => (p && p.left === left && p.top === top ? p : { left, top }))
  }, [card, profile, member, editing, error])

  const user = profile ?? member?.user ?? null
  const isSelf = card.userId === me?.id
  const isOwner = !!detail && detail.community.owner_id === card.userId
  const perms = detail?.permissions ?? 0
  const canEditNickname =
    !!member &&
    (isSelf
      ? has(perms, Permission.CHANGE_NICKNAME) || has(perms, Permission.MANAGE_NICKNAMES)
      : has(perms, Permission.MANAGE_NICKNAMES) && !isOwner)

  const roles: Role[] = member && detail
    ? sortRoles(detail.roles.filter((r) => !r.is_default && member.role_ids.includes(r.id)))
    : []
  const name = member?.nickname || displayName(user)

  async function saveNickname(e: FormEvent, value: string | null) {
    e.preventDefault()
    if (!card.communityId) return
    setError(null)
    try {
      const updated = await endpoints.setNickname(card.communityId, isSelf ? '@me' : card.userId, value)
      useStore.getState().upsertMember(card.communityId, updated)
      setEditing(false)
    } catch (err) {
      setError(errorText(err))
    }
  }

  return (
    <div
      ref={ref}
      className={`profile-card${profile?.accent_color != null ? ' has-accent' : ''}`}
      role="dialog"
      aria-label={`Profil von ${name}`}
      style={{
        ...accentStyle(profile?.accent_color),
        width: CARD_WIDTH,
        left: pos?.left ?? -9999,
        top: pos?.top ?? 0,
        visibility: pos ? 'visible' : 'hidden',
      }}
    >
      <BannerFill
        url={profile?.banner_url}
        color={profile?.banner_color}
        accent={profile?.accent_color}
        className="profile-card-banner"
      />
      <button type="button" className="icon-button small profile-card-close" onClick={close} aria-label="Schließen">
        <X size={14} />
      </button>
      <div className="profile-card-head">
        <Avatar
          user={user ?? { id: card.userId }}
          name={name}
          size={80}
          status={status}
          className="profile-card-avatar"
        />
      </div>
      <div className="profile-card-body">
        <div className="profile-card-names">
          <strong className="profile-card-name">
            {name}
            {isOwner && (
              <span className="member-owner" title="Besitzer">
                {' '}
                <Crown size={14} />
              </span>
            )}
          </strong>
          {user && (
            <span className="muted small">
              {member?.nickname && user.display_name ? `${user.display_name} · ` : ''}@{user.username}
              {profile?.pronouns && <span className="profile-pronouns"> · {profile.pronouns}</span>}
            </span>
          )}
        </div>
        {profile?.status_text && <p className="profile-card-status">{profile.status_text}</p>}

        {profile?.bio && (
          <section>
            <h4>Über mich</h4>
            <ProfileBio bio={profile.bio} />
          </section>
        )}

        {roles.length > 0 && (
          <section>
            <h4>Rollen</h4>
            <span className="role-chips">
              {roles.map((r) => (
                <span key={r.id} className="role-chip">
                  <span className="role-dot" style={{ background: roleColor(r.color) }} aria-hidden="true" />
                  {r.name}
                </span>
              ))}
            </span>
          </section>
        )}

        <section className="profile-card-dates">
          {member && (
            <span className="small">
              <span className="muted">Mitglied seit</span> {dateFmt.format(new Date(member.joined_at))}
            </span>
          )}
          {profile && (
            <span className="small">
              <span className="muted">Bei Lykos seit</span> {dateFmt.format(new Date(profile.created_at))}
            </span>
          )}
        </section>

        {canEditNickname &&
          (editing ? (
            <form className="profile-card-nickname" onSubmit={(e) => void saveNickname(e, nickname.trim() || null)}>
              <EmojiInput
                value={nickname}
                onChange={(e) => setNickname(e.target.value)}
                maxLength={32}
                placeholder={displayName(user)}
                aria-label="Nickname"
                autoFocus
              />
              <div className="form-actions">
                {member?.nickname && (
                  <button type="button" className="button" onClick={(e) => void saveNickname(e, null)}>
                    Entfernen
                  </button>
                )}
                <button type="button" className="button" onClick={() => setEditing(false)}>
                  Abbrechen
                </button>
                <button type="submit" className="button primary">
                  Speichern
                </button>
              </div>
            </form>
          ) : (
            <button
              type="button"
              className="link-button profile-card-edit"
              onClick={() => {
                setNickname(member?.nickname ?? '')
                setEditing(true)
              }}
            >
              <Pencil size={13} aria-hidden="true" /> {isSelf ? 'Eigenen Nickname ändern' : 'Nickname ändern'}
            </button>
          ))}
        {!isSelf && <FriendActions userId={card.userId} onDone={close} />}
        <button
          type="button"
          className="link-button profile-card-edit"
          onClick={() => {
            useProfileModal.getState().open(card.userId, card.communityId)
            close()
          }}
        >
          <Maximize2 size={13} aria-hidden="true" /> Vollständiges Profil
        </button>
        {error && <p className="error small">{error}</p>}
      </div>
    </div>
  )
}

/** Freundschafts-Schnellaktionen auf der Profilkarte (Hinzufügen, Annehmen, Nachricht …). */
function FriendActions({ userId, onDone }: { userId: string; onDone: () => void }) {
  const type = useRelationshipType(userId)
  const openDm = useStore((s) => s.openDm)
  const can = friendActions(type, false)
  const f = useFriends.getState()
  return (
    <div className="profile-card-friend">
      {type === 'friend' && (
        <button
          type="button"
          className="button primary small-button"
          onClick={() => runAction(() => openDm([userId]).then(onDone))}
        >
          <MessageCircle size={14} aria-hidden="true" /> Nachricht
        </button>
      )}
      {can.add && (
        <button type="button" className="button primary small-button" onClick={() => runAction(() => f.addFriend(userId))}>
          <UserPlus size={14} aria-hidden="true" /> Freund hinzufügen
        </button>
      )}
      {can.accept && (
        <button type="button" className="button primary small-button" onClick={() => runAction(() => f.addFriend(userId))}>
          <Check size={14} aria-hidden="true" /> Anfrage annehmen
        </button>
      )}
      {can.decline && (
        <button type="button" className="button small-button" onClick={() => runAction(() => f.remove(userId))}>
          <X size={14} aria-hidden="true" /> Ablehnen
        </button>
      )}
      {can.cancel && (
        <button type="button" className="button small-button" onClick={() => runAction(() => f.remove(userId))}>
          <X size={14} aria-hidden="true" /> Anfrage zurückziehen
        </button>
      )}
      {can.unblock && (
        <button type="button" className="button small-button" onClick={() => runAction(() => f.remove(userId))}>
          <UserCheck size={14} aria-hidden="true" /> Blockade aufheben
        </button>
      )}
    </div>
  )
}
