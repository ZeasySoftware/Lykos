/**
 * Profil-Darstellung (Paket J): Vorschau der Profilkarte (Einstellungen) und das große Profil-Modal
 * („Vollständiges Profil“: Banner, Avatar, Name, Pronomen, Über mich, Daten, gemeinsame Communities, Rollen).
 */
import { useEffect, useState } from 'react'
import type { PresenceStatus, Role, User, UserProfile } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { Markdown } from '../../lib/markdown'
import { sortRoles } from '../../lib/roles'
import { roleColor } from '../../lib/permissionLabels'
import { statusOf } from '../../lib/presence'
import { displayName, useMember, useStore } from '../../store'
import { Avatar } from '../Presence'
import { accentStyle } from '../../lib/customize'
import { BannerFill, CommunityIcon } from './parts'
import { useProfileModal } from './profileModal'
import { Crown, X } from 'lucide-react'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })

/** Felder, die Karte und Modal brauchen (eigenes Konto oder geladenes Profil). */
type ProfileLike = Pick<User, 'id' | 'username' | 'display_name' | 'avatar_url' | 'created_at'> &
  Partial<Pick<UserProfile, 'bio' | 'status_text' | 'pronouns' | 'banner_url' | 'banner_color' | 'accent_color'>>

/** Über mich mit Markdown light (gleicher Renderer wie Nachrichten, ohne Erwähnungen). */
export function ProfileBio({ bio }: { bio: string }) {
  return (
    <div className="profile-card-bio">
      <Markdown content={bio} />
    </div>
  )
}

/**
 * Statische Profilkarte für die Live-Vorschau in den Einstellungen – gleiche Gestaltung wie das Popover.
 */
export function ProfileCardPreview({ profile, status }: { profile: ProfileLike; status?: PresenceStatus }) {
  const name = displayName(profile)
  return (
    <div
      className={`profile-card profile-card-static${profile.accent_color != null ? ' has-accent' : ''}`}
      style={accentStyle(profile.accent_color)}
      aria-label="Vorschau der Profilkarte"
    >
      <BannerFill
        url={profile.banner_url}
        color={profile.banner_color}
        accent={profile.accent_color}
        className="profile-card-banner"
      />
      <div className="profile-card-head">
        <Avatar user={profile} name={name} size={80} status={status} className="profile-card-avatar" />
      </div>
      <div className="profile-card-body">
        <div className="profile-card-names">
          <strong className="profile-card-name">{name}</strong>
          <span className="muted small">
            @{profile.username}
            {profile.pronouns && <span className="profile-pronouns"> · {profile.pronouns}</span>}
          </span>
        </div>
        {profile.status_text && <p className="profile-card-status">{profile.status_text}</p>}
        {profile.bio && (
          <section>
            <h4>Über mich</h4>
            <ProfileBio bio={profile.bio} />
          </section>
        )}
        <section className="profile-card-dates">
          <span className="small">
            <span className="muted">Bei Lykos seit</span> {dateFmt.format(new Date(profile.created_at))}
          </span>
        </section>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Großes Profil-Modal
// ---------------------------------------------------------------------------

/** Einmal in der App einhängen; zeigt das Profil-Modal, wenn eines geöffnet ist. */
export function ProfileModalHost() {
  const target = useProfileModal((s) => s.target)
  if (!target) return null
  return <ProfileModal key={`${target.userId}:${target.communityId}`} {...target} />
}

function ProfileModal({ userId, communityId }: { userId: string; communityId: string | null }) {
  const close = useProfileModal((s) => s.close)
  const loadProfile = useStore((s) => s.loadProfile)
  const profile = useStore((s) => s.profiles[userId])
  const member = useMember(communityId, userId)
  const detail = useStore((s) => (communityId ? s.details[communityId] : undefined))
  const communities = useStore((s) => s.communities)
  const selectCommunity = useStore((s) => s.selectCommunity)
  const status = useStore((s) => statusOf(s, userId))
  const [error, setError] = useState<string | null>(null)

  // Immer frisch laden: liefert auch die gemeinsamen Communities (nicht Teil von USER_UPDATE).
  useEffect(() => {
    loadProfile(userId).catch((e) => setError(errorText(e)))
  }, [userId, loadProfile])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [close])

  const user = profile ?? member?.user ?? null
  const name = member?.nickname || displayName(user)
  const isOwner = !!detail && detail.community.owner_id === userId
  const roles: Role[] =
    member && detail
      ? sortRoles(detail.roles.filter((r) => !r.is_default && member.role_ids.includes(r.id)))
      : []
  const mutual = (profile?.mutual_community_ids ?? [])
    .map((id) => communities.find((c) => c.id === id))
    .filter((c) => c !== undefined)

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        className={`profile-modal${profile?.accent_color != null ? ' has-accent' : ''}`}
        role="dialog"
        aria-modal="true"
        aria-label={`Profil von ${name}`}
        style={accentStyle(profile?.accent_color)}
      >
        <BannerFill
          url={profile?.banner_url}
          color={profile?.banner_color}
          accent={profile?.accent_color}
          className="profile-modal-banner"
        />
        <button type="button" className="icon-button profile-card-close" onClick={close} aria-label="Schließen">
          <X size={18} />
        </button>
        <div className="profile-modal-head">
          <Avatar user={user ?? { id: userId }} name={name} size={120} status={status} className="profile-card-avatar" />
        </div>
        <div className="profile-modal-body">
          <div className="profile-modal-names">
            <h2 className="profile-modal-name">
              {name}
              {isOwner && (
                <span className="member-owner" title="Besitzer">
                  {' '}
                  <Crown size={18} />
                </span>
              )}
            </h2>
            {user && (
              <span className="muted">
                {member?.nickname && user.display_name ? `${user.display_name} · ` : ''}@{user.username}
                {profile?.pronouns && <span className="profile-pronouns"> · {profile.pronouns}</span>}
              </span>
            )}
            {profile?.status_text && <p className="profile-card-status">{profile.status_text}</p>}
          </div>

          <div className="profile-modal-grid">
            <div className="profile-modal-col">
              <section>
                <h4>Über mich</h4>
                {profile?.bio ? <ProfileBio bio={profile.bio} /> : <p className="muted small">Noch nichts geschrieben.</p>}
              </section>
              {roles.length > 0 && (
                <section>
                  <h4>Rollen{detail ? ` in ${detail.community.name}` : ''}</h4>
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
            </div>
            <div className="profile-modal-col">
              <section className="profile-modal-dates">
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
              {mutual.length > 0 && (
                <section>
                  <h4>Gemeinsame Communities ({mutual.length})</h4>
                  <ul className="mutual-list">
                    {mutual.map((c) => (
                      <li key={c.id}>
                        <button
                          type="button"
                          className="mutual-item"
                          onClick={() => {
                            selectCommunity(c.id)
                            close()
                          }}
                        >
                          <CommunityIcon community={c} className="mutual-icon" />
                          <span className="truncate">{c.name}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </div>
          </div>
          {error && <p className="error small">{error}</p>}
        </div>
      </div>
    </div>
  )
}
