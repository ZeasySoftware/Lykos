import { useState } from 'react'
import { isDesktopBundle } from '../lib/api'
import type { Community, CommunityDetail } from '../lib/api'
import { buildCommunityMenu } from '../lib/contextMenus'
import { useContextMenu } from '../lib/menuStore'
import { isCommunityMuted, useMutesVersion } from '../lib/notify'
import { communityMentionCount, isCommunityUnread, isDmUnread } from '../lib/unread'
import { useStore } from '../store'
import { incomingCount } from '../lib/friends'
import { useFriends } from '../lib/friendsStore'
import { LykosMark } from './Brand'
import { AddCommunityDialog } from './Dialogs'
import { InstanceAdminRailButton } from './admin/InstanceAdmin'
import { Download, MessageCircle, Plus } from 'lucide-react'
import { accentStyle } from '../lib/customize'
import { CommunityIcon } from './customize/parts'

/** Kachel einer Community; Rechtsklick öffnet ihr Kontextmenü. */
function RailCommunity({
  community,
  detail,
  active,
  onSelect,
}: {
  community: Community
  detail: CommunityDetail | undefined
  active: boolean
  onSelect: () => void
}) {
  const menu = useContextMenu(() => buildCommunityMenu(community.id, 'rail'), community.name)
  useMutesVersion()
  const mentions = communityMentionCount(detail)
  const muted = isCommunityMuted(community.id)
  return (
    <button
      type="button"
      className={`rail-item${active ? ' active' : ''}${muted ? ' muted-community' : ''}${community.icon_url ? ' has-icon' : ''}${community.accent_color != null ? ' has-accent' : ''}`}
      style={accentStyle(community.accent_color)}
      title={community.name}
      aria-label={community.name}
      aria-current={active}
      onClick={onSelect}
      {...menu}
    >
      <CommunityIcon community={community} />
      {mentions > 0 ? (
        <span className="rail-mentions" aria-label={`${mentions} ungelesene Erwähnungen`}>
          {mentions > 99 ? '99+' : mentions}
        </span>
      ) : (
        !active &&
        !muted &&
        isCommunityUnread(detail) && <span className="rail-unread" aria-label="Ungelesene Nachrichten" />
      )}
    </button>
  )
}

export function CommunityRail() {
  const communities = useStore((s) => s.communities)
  const selected = useStore((s) => s.selectedCommunityId)
  const selectCommunity = useStore((s) => s.selectCommunity)
  const details = useStore((s) => s.details)
  const dmView = useStore((s) => s.dmView)
  const openDmView = useStore((s) => s.openDmView)
  const dmUnread = useStore((s) => Object.values(s.dms).some((dm) => dm.id !== s.selectedDmId && isDmUnread(dm)))
  const dmMentions = useStore((s) => Object.values(s.dms).reduce((sum, dm) => sum + (dm.mention_count ?? 0), 0))
  // Offene Freundschaftsanfragen zählen wie Erwähnungen in den Zähler der DM-Kachel.
  const friendRequests = useFriends((s) => incomingCount(Object.values(s.relationships)))
  const dmBadge = dmMentions + friendRequests
  const [adding, setAdding] = useState(false)

  return (
    <nav className="rail" aria-label="Communities">
      <LykosMark className="rail-logo" title="Lykos" />
      <button
        type="button"
        className={`rail-item rail-dms${dmView ? ' active' : ''}`}
        title="Direktnachrichten"
        aria-label="Direktnachrichten"
        aria-current={dmView}
        onClick={openDmView}
      >
        <MessageCircle size={22} />
        {dmBadge > 0 ? (
          <span
            className="rail-mentions"
            aria-label={
              friendRequests > 0
                ? `${dmMentions} ungelesene Erwähnungen, ${friendRequests} Freundschaftsanfragen`
                : `${dmMentions} ungelesene Erwähnungen`
            }
          >
            {dmBadge > 99 ? '99+' : dmBadge}
          </span>
        ) : (
          dmUnread && <span className="rail-unread" aria-label="Ungelesene Direktnachrichten" />
        )}
      </button>
      <div className="rail-divider" />
      <div className="rail-list">
        {communities.map((c) => (
          <RailCommunity
            key={c.id}
            community={c}
            detail={details[c.id]}
            active={c.id === selected && !dmView}
            onSelect={() => selectCommunity(c.id)}
          />
        ))}
      </div>
      <button className="rail-add" type="button" title="Community hinzufügen" onClick={() => setAdding(true)}>
        <Plus size={24} />
      </button>
      {/* Nur im Browser: immer erreichbarer Weg zur Desktop-App (neuer Tab, die Sitzung bleibt offen). */}
      {!isDesktopBundle && (
        <a
          className="rail-add rail-download"
          href="/download"
          target="_blank"
          rel="noopener"
          title="Desktop-App installieren"
          aria-label="Desktop-App installieren"
        >
          <Download size={22} />
        </a>
      )}
      <InstanceAdminRailButton />
      {adding && <AddCommunityDialog onClose={() => setAdding(false)} />}
    </nav>
  )
}
