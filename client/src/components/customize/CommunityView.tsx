/**
 * Community-Darstellung (Paket J): Banner im Kanalleisten-Kopf, Info-Popover mit Beschreibung,
 * Einladungskarte und Systemnachrichten („… ist beigetreten“).
 */
import { buildSystemMessageMenu, buildUserMenu, deleteMessage } from '../../lib/contextMenus'
import { useContextMenu } from '../../lib/menuStore'
import type { Community, InvitePreview, Message } from '../../lib/api'
import { useMemberName, useStore } from '../../store'
import { accentStyle } from '../../lib/customize'
import { BannerFill, CommunityIcon } from './parts'
import { Trash2, UserPlus } from 'lucide-react'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })
const timeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/** Hintergrund des Kanalleisten-Kopfs (Banner oder Verlauf aus der Akzentfarbe); sonst nichts. */
export function SidebarHeaderBanner({ community }: { community: Community | undefined }) {
  if (!community || (!community.banner_url && community.accent_color == null)) return null
  return <BannerFill url={community.banner_url} accent={community.accent_color} className="sidebar-banner" />
}

/** Kopf des Community-Dropdowns: Banner, Icon, Name, Beschreibung. */
export function CommunityInfoCard({ community }: { community: Community }) {
  const members = useStore((s) => s.members[community.id])
  return (
    <div className="community-popover community-info-card">
      <BannerFill url={community.banner_url} accent={community.accent_color} className="community-popover-banner" />
      <div className="community-popover-body">
        <CommunityIcon community={community} className="community-popover-icon" style={accentStyle(community.accent_color)} />
        <strong className="community-popover-name">{community.name}</strong>
        {community.description ? (
          <p className="community-popover-description">{community.description}</p>
        ) : (
          <p className="muted small">Keine Beschreibung.</p>
        )}
        <span className="muted small">
          {members ? `${members.length} Mitglieder · ` : ''}Seit {dateFmt.format(new Date(community.created_at))}
        </span>
      </div>
    </div>
  )
}

/** Einladungskarte (Einladungsseite, Beitrittsdialog, Vorschau in den Einstellungen). */
/**
 * Einladungskarte für die Einladungsseite und den Beitrittsdialog – eine Komponente für beide Arten:
 * Community-Einladung („Du wurdest eingeladen zu <Community>“, mit Icon/Banner/Beschreibung) und
 * Instanz-Einladung („Du wurdest eingeladen zu <Instanz>“, ggf. mit automatischem Community-Beitritt).
 */
export function InviteCard({
  preview,
}: {
  preview: Pick<InvitePreview, 'community' | 'member_count' | 'kind' | 'instance_name'>
}) {
  const c = preview.community
  const instance = preview.kind === 'instance'
  if (!c) {
    // Instanz-Einladung ohne Community.
    return (
      <div className="invite-card">
        <div className="invite-card-body">
          <span className="muted small">Du wurdest eingeladen zu</span>
          <strong className="invite-card-name">{preview.instance_name ?? 'Lykos'}</strong>
        </div>
      </div>
    )
  }
  return (
    <div className={`invite-card${c.accent_color != null ? ' has-accent' : ''}`} style={accentStyle(c.accent_color)}>
      <BannerFill url={c.banner_url} accent={c.accent_color} className="invite-card-banner" />
      <div className="invite-card-body">
        <CommunityIcon community={c} className="invite-card-icon" />
        <span className="muted small">Du wurdest eingeladen zu</span>
        <strong className="invite-card-name">{instance ? (preview.instance_name ?? 'Lykos') : c.name}</strong>
        {instance && <span className="muted small">und trittst automatisch „{c.name}“ bei</span>}
        {c.description && <p className="invite-card-description">{c.description}</p>}
        <span className="muted small">{preview.member_count} Mitglieder</span>
      </div>
    </div>
  )
}

const JOIN_TEXTS: [string, string][] = [
  ['', ' ist beigetreten.'],
  ['Willkommen, ', '!'],
  ['', ' ist zum Rudel gestoßen.'],
]

/** Systemnachricht im Verlauf: Beitritt eines Mitglieds (`kind = member_join`). */
export function SystemMessage({ message, canDelete }: { message: Message; canDelete: boolean }) {
  const openProfile = useStore((s) => s.openProfile)
  const name = useMemberName(message.community_id, message.author)
  const menu = useContextMenu(() => buildSystemMessageMenu(message, canDelete), 'Systemnachricht')
  const authorMenu = useContextMenu(
    (ctx) =>
      message.author
        ? buildUserMenu({ userId: message.author.id, communityId: message.community_id, target: ctx.target })
        : [],
    name,
  )
  // Gleiche Nachricht → gleicher Text auf allen Geräten.
  const [before, after] = JOIN_TEXTS[message.id.charCodeAt(message.id.length - 1) % JOIN_TEXTS.length]
  return (
    <div className="message system-message" data-message-id={message.id} {...menu}>
      <UserPlus size={16} className="system-message-icon" aria-hidden="true" />
      <span className="system-message-text">
        {before}
        {message.author ? (
          <button
            type="button"
            className="message-author"
            data-menu-anchor
            {...authorMenu}
            onClick={(e) => openProfile(message.author!.id, message.community_id, e.currentTarget.getBoundingClientRect())}
          >
            {name}
          </button>
        ) : (
          <span className="message-author deleted">{name}</span>
        )}
        {after}
      </span>
      <time className="message-time" dateTime={message.created_at}>
        {timeFmt.format(new Date(message.created_at))}
      </time>
      {canDelete && (
        <div className="message-actions">
          <button
            type="button"
            className="icon-button small danger"
            title="Meldung entfernen"
            aria-label="Meldung entfernen"
            onClick={(e) => deleteMessage(message, e.shiftKey)}
          >
            <Trash2 size={15} />
          </button>
        </div>
      )}
    </div>
  )
}
