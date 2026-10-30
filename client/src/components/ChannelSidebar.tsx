import type { ReactNode } from 'react'
import type { Category, Channel, ChannelKind, CommunityDetail } from '../lib/api'
import { sidebarLayout } from '../lib/categories'
import type { ChannelSection } from '../lib/categories'
import { isCategoryCollapsed, toggleCategoryCollapsed, useCollapsedVersion } from '../lib/categoryCollapse'
import { buildCategoryMenu, buildChannelMenu, buildCommunityMenu, buildSelfMenu } from '../lib/contextMenus'
import { openDialog } from '../lib/dialogs'
import { useContextMenu, useDropdown } from '../lib/menuStore'
import { isChannelMuted, useMutesVersion } from '../lib/notify'
import { Permission, has } from '../lib/permissions'
import { isChannelUnread } from '../lib/unread'
import { useStore } from '../store'
import { FeedbackButton } from './feedback/FeedbackHub'
import { StatusPicker } from './Presence'
import { UpdateIndicator } from './UpdateIndicator'
import { VoiceChannel } from './VoiceChannel'
import { VoicePanel } from './VoicePanel'
import { sidebarHeaderClass } from '../lib/customize'
import { SidebarHeaderBanner } from './customize/CommunityView'
import { ChevronDown, ChevronRight, Hash, Plus, Settings, Volume2 } from 'lucide-react'

export function ChannelSidebar({ detail }: { detail: CommunityDetail | undefined }) {
  const selectedChannelId = useStore((s) => s.selectedChannelId)
  const selectChannel = useStore((s) => s.selectChannel)
  useMutesVersion()
  useCollapsedVersion()

  const perms = detail?.permissions ?? 0
  const canManageChannels = has(perms, Permission.MANAGE_CHANNELS)
  const layout = sidebarLayout(detail?.channels ?? [], detail?.categories, canManageChannels)
  const { text, voice } = layout.uncategorized
  /** Zahnrad am Kanal, wenn man ihn bearbeiten oder seine Berechtigungen ändern darf. */
  const settingsFor = (c: Channel) => {
    const p = detail?.channel_permissions[c.id] ?? perms
    return has(p, Permission.MANAGE_CHANNELS) ||
      has(p, Permission.MANAGE_ROLES) ||
      (c.kind === 'text' && has(p, Permission.MANAGE_WEBHOOKS))
      ? () => openDialog({ kind: 'channelSettings', communityId: c.community_id, channelId: c.id })
      : undefined
  }
  const addChannel = (kind: ChannelKind, categoryId?: string) =>
    canManageChannels && detail
      ? () => openDialog({ kind: 'createChannel', communityId: detail.community.id, channelKind: kind, categoryId })
      : undefined
  /** Kanäle einer Gruppe: Text vor Sprache, jeweils nach Position. */
  const renderChannels = (section: ChannelSection) => (
    <>
      {section.text.map((c) =>
        detail ? (
          <ChannelButton
            key={c.id}
            channel={c}
            active={c.id === selectedChannelId}
            unread={isChannelUnread(detail, c.id)}
            mentions={detail.mention_counts?.[c.id] ?? 0}
            onClick={() => selectChannel(c.id)}
            onSettings={settingsFor(c)}
          />
        ) : null,
      )}
      {section.voice.map((c) =>
        detail ? <VoiceChannel key={c.id} channel={c} detail={detail} onSettings={settingsFor(c)} /> : null,
      )}
    </>
  )
  // Ohne Kategorien wie bisher: „Textkanäle“ immer, „Sprachkanäle“ bei Kanälen oder Verwaltungsrecht.
  // Mit Kategorien fallen leere obere Gruppen weg (die Kanäle stehen dann in den Kategorien).
  const hasCategories = layout.categories.length > 0
  const showText = !hasCategories || text.length > 0
  const showVoice = voice.length > 0 || (canManageChannels && !hasCategories)

  return (
    <aside className="sidebar">
      {detail ? (
        <CommunityHeader detail={detail} />
      ) : (
        <header className="sidebar-header">
          <span className="truncate">Lykos</span>
        </header>
      )}

      <div className="channel-list">
        {!detail && <p className="sidebar-empty">Lege eine Community an oder tritt einer bei.</p>}
        {detail && (
          <>
            {showText && (
              <ChannelGroup title="Textkanäle" onAdd={addChannel('text')}>
                {renderChannels({ text, voice: [] })}
                {text.length === 0 && <p className="sidebar-empty">Keine Textkanäle.</p>}
              </ChannelGroup>
            )}
            {showVoice && (
              <ChannelGroup title="Sprachkanäle" onAdd={addChannel('voice')}>
                {renderChannels({ text: [], voice })}
                {voice.length === 0 && <p className="sidebar-empty">Keine Sprachkanäle.</p>}
              </ChannelGroup>
            )}
            {layout.categories.map((section) => (
              <CategoryGroup
                key={section.category.id}
                category={section.category}
                canManage={canManageChannels}
                collapsedContent={renderChannels({
                  text: section.text.filter((c) => c.id === selectedChannelId),
                  voice: [],
                })}
                onAdd={addChannel('text', section.category.id)}
              >
                {renderChannels(section)}
                {section.text.length + section.voice.length === 0 && (
                  <p className="sidebar-empty">Noch keine Kanäle.</p>
                )}
              </CategoryGroup>
            ))}
          </>
        )}
      </div>

    </aside>
  )
}

/** Community-Name oben: Klick öffnet das Dropdown (wie Discord), Rechtsklick dasselbe als Kontextmenü. */
function CommunityHeader({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const dropdown = useDropdown(() => buildCommunityMenu(communityId, 'header'), {
    label: `${detail.community.name} – Menü`,
    matchWidth: true,
  })
  const menu = useContextMenu(() => buildCommunityMenu(communityId, 'header'), detail.community.name)
  return (
    <header className={`sidebar-header${sidebarHeaderClass(detail.community)}`}>
      <SidebarHeaderBanner community={detail.community} />
      <button type="button" className="community-header-button" {...dropdown} onContextMenu={menu.onContextMenu}>
        <span className="truncate">{detail.community.name}</span>
        <ChevronDown size={18} className="community-header-chevron" aria-hidden="true" />
      </button>
    </header>
  )
}

function ChannelGroup({
  title,
  onAdd,
  children,
}: {
  title: string
  onAdd?: () => void
  children: ReactNode
}) {
  return (
    <section className="channel-group">
      <header>
        <span>{title}</span>
        {onAdd && (
          <button type="button" className="icon-button small" title="Kanal erstellen" aria-label={`${title}: Kanal erstellen`} onClick={onAdd}>
            <Plus size={15} />
          </button>
        )}
      </header>
      {children}
    </section>
  )
}

/**
 * Kategorie: Kopf mit Pfeil (Klick klappt ein/aus, Zustand lokal gespeichert), Kontextmenü und Zahnrad/Plus
 * für Kanal-Verwalter. Eingeklappt bleibt der gerade offene Kanal sichtbar (wie bei Discord).
 */
function CategoryGroup({
  category,
  canManage,
  collapsedContent,
  onAdd,
  children,
}: {
  category: Category
  canManage: boolean
  /** Inhalt im eingeklappten Zustand (der offene Kanal). */
  collapsedContent?: ReactNode
  onAdd?: () => void
  children: ReactNode
}) {
  const collapsed = isCategoryCollapsed(category.id)
  const menu = useContextMenu(() => buildCategoryMenu(category), `Kategorie ${category.name}`)
  const settings = useDropdown(() => buildCategoryMenu(category), { label: `${category.name} – Menü` })
  const bodyId = `category-${category.id}`
  return (
    <section className={`channel-group category-group${collapsed ? ' collapsed' : ''}`}>
      <header>
        <button
          type="button"
          className="category-toggle"
          aria-expanded={!collapsed}
          aria-controls={bodyId}
          onClick={() => toggleCategoryCollapsed(category.id)}
          {...menu}
        >
          {collapsed ? <ChevronRight size={12} aria-hidden="true" /> : <ChevronDown size={12} aria-hidden="true" />}
          <span className="truncate">{category.name}</span>
        </button>
        {canManage && (
          <span className="category-actions">
            <button type="button" className="icon-button small" title="Kategorie bearbeiten" aria-label={`${category.name}: Kategorie bearbeiten`} {...settings}>
              <Settings size={14} />
            </button>
            {onAdd && (
              <button type="button" className="icon-button small" title="Kanal hier erstellen" aria-label={`${category.name}: Kanal erstellen`} onClick={onAdd}>
                <Plus size={15} />
              </button>
            )}
          </span>
        )}
      </header>
      <div id={bodyId} className="category-body">
        {collapsed ? collapsedContent : children}
      </div>
    </section>
  )
}

function ChannelButton({
  channel,
  active,
  disabled,
  unread,
  mentions = 0,
  onClick,
  onSettings,
}: {
  channel: Channel
  active: boolean
  disabled?: boolean
  unread?: boolean
  /** Ungelesene Erwähnungen (Badge). */
  mentions?: number
  onClick?: () => void
  onSettings?: () => void
}) {
  const menu = useContextMenu(() => buildChannelMenu(channel), `Kanal ${channel.name}`)
  // Stummgeschaltete Kanäle treten zurück; Ungelesenes zeigen sie nicht hervorgehoben (Erwähnungen schon).
  const muted = isChannelMuted(channel.id)
  const classes = ['channel']
  if (active) classes.push('active')
  if (unread && !muted) classes.push('unread')
  if (muted) classes.push('muted-channel')
  const button = (
    <button
      type="button"
      className={classes.join(' ')}
      aria-label={unread ? `${channel.name} (ungelesen)` : undefined}
      disabled={disabled}
      onClick={onClick}
      title={channel.topic ?? channel.name}
      {...menu}
    >
      <span className="channel-icon" aria-hidden="true">
        {channel.kind === 'text' ? <Hash size={18} /> : <Volume2 size={18} />}
      </span>
      <span className="truncate">{channel.name}</span>
      {mentions > 0 && !active && (
        <span className="mention-badge" aria-label={`${mentions} ungelesene Erwähnungen`}>
          {mentions > 99 ? '99+' : mentions}
        </span>
      )}
    </button>
  )
  if (!onSettings) return button
  return (
    <div className="channel-row">
      {button}
      <button type="button" className="channel-settings icon-button small" title="Kanal bearbeiten" onClick={onSettings}>
        <Settings size={15} />
      </button>
    </div>
  )
}

/**
 * Konto-Block unten links (wie Discord): schwebt über Community-Rail und Kanalleiste, oben die
 * Voice-Leiste (falls verbunden), darunter Avatar, Name, Status und Schaltflächen. Rechtsklick
 * öffnet das Konto-Menü. Liegt im Shell-Raster in der Zeile `dock` unter Rail und Seitenleiste.
 */
export function AccountDock() {
  return (
    <div className="account-dock-area">
      <section className="account-dock" aria-label="Konto und Sprachverbindung">
        <VoicePanel />
        <UserPanel />
      </section>
    </div>
  )
}

export function UserPanel() {
  const menu = useContextMenu(buildSelfMenu, 'Eigenes Konto')
  return (
    <footer className="user-panel" onContextMenu={menu.onContextMenu}>
      {/* Avatar, Name und Menü (Status, Status-Text, Profil, Abmelden) */}
      <StatusPicker />
      <UpdateIndicator />
      <FeedbackButton />
      <button
        type="button"
        className="icon-button"
        title="Einstellungen (Strg+,)"
        aria-label="Einstellungen"
        onClick={() => openDialog({ kind: 'settings' })}
      >
        <Settings size={18} />
      </button>
    </footer>
  )
}
