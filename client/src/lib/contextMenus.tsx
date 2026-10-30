/**
 * Einträge der Kontextmenüs (Nachricht, Nutzer, Kanal, Community, DM, eigene Nutzerleiste).
 * Die Builder lesen den aktuellen Zustand per `getState()` – sie laufen bei jedem Rendern des
 * offenen Menüs neu, Haken und Rechte sind also immer aktuell. Rechte-Regeln: `menuRules.ts`.
 */
import {
  AtSign,
  Bell,
  BellOff,
  CheckCheck,
  Copy,
  Download,
  ExternalLink,
  IdCard,
  Image,
  Link,
  LogOut,
  MessageCircle,
  Pencil,
  Phone,
  PhoneCall,
  PhoneOff,
  Pin,
  PinOff,
  Plus,
  Reply,
  Settings,
  SmilePlus,
  Trash2,
  UserPen,
  UserPlus,
  UserRound,
  UserX,
  CircleX,
  Eye,
  EyeOff,
  Focus,
  Maximize,
  Minimize2,
  Ban,
  Check,
  UserCheck,
  UserMinus,
  X,
} from 'lucide-react'
import { endpoints } from './api'
import type { Attachment, Channel, DmChannel, Message, PresenceStatus } from './api'
import { useAppSettings } from './appSettings'
import { confirmAction, openDialog } from './dialogs'
import type { CommunityTab } from './dialogs'
import { absoluteAttachmentUrl, downloadAttachment } from './download'
import { openExternalUrl } from './external'
import type { CallTile } from './callLayout'
import type { ActionItem, MenuItem } from './menu'
import { channelRules, communityRules, communityTabs, memberRules, messageRules, roleOptions } from './menuRules'
import {
  MUTE_DURATIONS,
  isChannelMuted,
  isCommunityMuted,
  muteUntilLabel,
  setChannelMuted,
  setCommunityMuted,
} from './notify'
import { Permission, has } from './permissions'
import { roleColor } from './permissionLabels'
import { choosableStatuses, presenceLabel } from './presence'
import { copyText, runAction } from './toast'
import { isChannelUnread, isCommunityUnread, isDmUnread } from './unread'
import { DEFAULT_USER_AUDIO, useVoice } from './voice'
import { callUser, startCall } from './callActions'
import { friendActions } from './friends'
import { addFriendWithFeedback, useFriends } from './friendsStore'
import { CommunityInfoCard } from '../components/customize/CommunityView'
import { QuickReactions, VolumeSlider } from '../components/MenuWidgets'
import { channelLabel, displayName, dmTitle, memberName, useStore } from '../store'

const ICON = 16

function devMode(): boolean {
  return useAppSettings.getState().developerMode
}

function copyIdItem(id: string, label: string): ActionItem {
  return {
    type: 'item',
    id: 'copy-id',
    label,
    icon: <IdCard size={ICON} />,
    hidden: !devMode(),
    onSelect: () => void copyText(id, 'ID kopiert'),
  }
}

/** „Stummschalten ›“ mit Dauer bzw. „Stummschaltung aufheben“. */
function muteItem(target: { channelId: string } | { communityId: string }, label: string): MenuItem {
  const isCommunity = 'communityId' in target
  const id = isCommunity ? target.communityId : target.channelId
  const muted = isCommunity ? isCommunityMuted(id) : isChannelMuted(id)
  const set = (d: number | null | false) => (isCommunity ? setCommunityMuted(id, d) : setChannelMuted(id, d))
  if (muted) {
    return {
      type: 'item',
      id: 'unmute',
      label: 'Stummschaltung aufheben',
      hint: muteUntilLabel(isCommunity ? { communityId: id } : { channelId: id })?.replace('Stumm ', '') ?? undefined,
      icon: <Bell size={ICON} />,
      onSelect: () => set(false),
    }
  }
  return {
    type: 'item',
    id: 'mute',
    label: `${label} stummschalten`,
    icon: <BellOff size={ICON} />,
    submenu: MUTE_DURATIONS.map((d) => ({
      type: 'item' as const,
      id: `mute-${d.ms ?? 'forever'}`,
      label: d.label,
      onSelect: () => set(d.ms),
    })),
  }
}

/** Bildschirmrechteck des Elements, an dem ein Menü geöffnet wurde (für die Profilkarte). */
function anchorOf(target: HTMLElement): DOMRect {
  return (target.closest('[data-menu-anchor]') ?? target).getBoundingClientRect()
}

// ---------------------------------------------------------------------------
// Nachricht
// ---------------------------------------------------------------------------

export interface MessageMenuOptions {
  message: Message
  /** Effektive Rechte im Kanal. */
  perms: number
  /** Zielelement des Rechtsklicks (Link? Bild?). */
  target: HTMLElement
  onReply: () => void
  onEdit: () => void
  /** Emoji-Auswahl an der Nachricht öffnen („Weitere…“). */
  onMoreReactions: () => void
  /** Bild in der Großansicht öffnen. */
  onOpenAttachment: (a: Attachment) => void
}

/** Pinnen/Lösen mit Fehler-Toast. */
export function togglePin(message: Message) {
  runAction(() =>
    message.pinned_at ? endpoints.unpin(message.channel_id, message.id) : endpoints.pin(message.channel_id, message.id),
  )
}

/** Nachricht löschen – mit Bestätigung (Umschalt überspringt sie wie bei Discord). */
export function deleteMessage(message: Message, skipConfirm = false) {
  const remove = () => useStore.getState().deleteMessage(message.id)
  if (skipConfirm) {
    runAction(remove)
    return
  }
  const preview = message.content.length > 160 ? `${message.content.slice(0, 160)} …` : message.content
  confirmAction({
    title: 'Nachricht löschen',
    message: (
      <>
        <p>Willst du diese Nachricht wirklich löschen?</p>
        {preview && <blockquote className="confirm-quote">{preview}</blockquote>}
        <p className="muted small">Tipp: Halte Umschalt beim Klick auf den Papierkorb, um ohne Nachfrage zu löschen.</p>
      </>
    ),
    confirmLabel: 'Löschen',
    action: remove,
  })
}

export function buildMessageMenu(o: MessageMenuOptions): MenuItem[] {
  const s = useStore.getState()
  const message = s.messages[o.message.channel_id]?.items.find((m) => m.id === o.message.id) ?? o.message
  const own = message.author?.id === s.user?.id
  const rules = messageRules({ own, perms: o.perms, isDm: message.community_id === null })

  const link = o.target.closest<HTMLAnchorElement>('a.md-link')
  const attachmentEl = o.target.closest<HTMLElement>('[data-attachment-id]')
  const attachment = attachmentEl
    ? message.attachments.find((a) => a.id === attachmentEl.dataset.attachmentId)
    : undefined
  const isImage = attachmentEl?.dataset.attachmentKind === 'image'

  const items: MenuItem[] = [
    {
      type: 'custom',
      id: 'quick-reactions',
      focusable: true,
      hidden: !rules.react,
      render: () => <QuickReactions message={message} onMore={o.onMoreReactions} />,
    },
    {
      type: 'item',
      id: 'react',
      label: 'Reaktion hinzufügen',
      icon: <SmilePlus size={ICON} />,
      hidden: !rules.react,
      onSelect: o.onMoreReactions,
    },
    { type: 'separator' },
    { type: 'item', id: 'edit', label: 'Nachricht bearbeiten', icon: <Pencil size={ICON} />, hidden: !rules.edit, onSelect: o.onEdit },
    { type: 'item', id: 'reply', label: 'Antworten', icon: <Reply size={ICON} />, hidden: !rules.reply, onSelect: o.onReply },
    {
      type: 'item',
      id: 'pin',
      label: message.pinned_at ? 'Nachricht lösen' : 'Nachricht anpinnen',
      icon: message.pinned_at ? <PinOff size={ICON} /> : <Pin size={ICON} />,
      hidden: !rules.pin,
      onSelect: () => togglePin(message),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'copy-text',
      label: 'Text kopieren',
      icon: <Copy size={ICON} />,
      hidden: !message.content,
      onSelect: () => void copyText(message.content, 'Text kopiert'),
    },
  ]

  if (link) {
    const url = link.href
    items.push(
      { type: 'separator' },
      { type: 'item', id: 'open-link', label: 'Link öffnen', icon: <ExternalLink size={ICON} />, onSelect: () => void openExternalUrl(url) },
      { type: 'item', id: 'copy-link', label: 'Link kopieren', icon: <Link size={ICON} />, onSelect: () => void copyText(url, 'Link kopiert') },
    )
  }

  if (attachment) {
    items.push(
      { type: 'separator' },
      {
        type: 'item',
        id: 'open-attachment',
        label: 'Bild öffnen',
        icon: <Image size={ICON} />,
        hidden: !isImage,
        onSelect: () => o.onOpenAttachment(attachment),
      },
      {
        type: 'item',
        id: 'download',
        label: isImage ? 'Bild speichern' : 'Datei herunterladen',
        icon: <Download size={ICON} />,
        onSelect: () => runAction(() => downloadAttachment(attachment)),
      },
      {
        type: 'item',
        id: 'copy-attachment-link',
        label: isImage ? 'Bildadresse kopieren' : 'Dateiadresse kopieren',
        icon: <Link size={ICON} />,
        onSelect: () => void copyText(absoluteAttachmentUrl(attachment), 'Adresse kopiert'),
      },
    )
  }

  items.push(
    { type: 'separator' },
    copyIdItem(message.id, 'Nachrichten-ID kopieren'),
    {
      type: 'item',
      id: 'delete',
      label: 'Nachricht löschen',
      icon: <Trash2 size={ICON} />,
      danger: true,
      hidden: !rules.delete,
      onSelect: () => deleteMessage(message),
    },
  )
  return items
}

/** Reduziertes Menü für Systemnachrichten („… ist beigetreten“). */
export function buildSystemMessageMenu(message: Message, canDelete: boolean): MenuItem[] {
  return [
    copyIdItem(message.id, 'Nachrichten-ID kopieren'),
    {
      type: 'item',
      id: 'delete',
      label: 'Meldung entfernen',
      icon: <Trash2 size={ICON} />,
      danger: true,
      hidden: !canDelete,
      onSelect: () => deleteMessage(message),
    },
  ]
}

// ---------------------------------------------------------------------------
// Nutzer (Name/Avatar in Nachrichten, Mitgliederliste, Voice, DMs)
// ---------------------------------------------------------------------------

export interface UserMenuOptions {
  userId: string
  /** Community, in deren Kontext das Menü geöffnet wurde (`null` = DM). */
  communityId: string | null
  target: HTMLElement
  /** Voice-Teilnehmer: lokale Lautstärke und Stummschaltung anbieten. */
  voice?: boolean
}

/** Offener Text-/DM-Kanal, in dessen Composer „Erwähnen“ schreiben würde – falls der Nutzer dort ist. */
function mentionTarget(userId: string): string | null {
  const s = useStore.getState()
  if (s.dmView) {
    const dm = s.selectedDmId ? s.dms[s.selectedDmId] : undefined
    return dm?.recipients.some((u) => u.id === userId) ? dm.id : null
  }
  const detail = s.selectedCommunityId ? s.details[s.selectedCommunityId] : undefined
  const channel = detail?.channels.find((c) => c.id === s.selectedChannelId && c.kind === 'text')
  if (!detail || !channel) return null
  const perms = detail.channel_permissions[channel.id] ?? detail.permissions
  if (!has(perms, Permission.SEND_MESSAGES)) return null
  const members = s.members[detail.community.id]
  return !members || members.some((m) => m.user.id === userId) ? channel.id : null
}

export function buildUserMenu(o: UserMenuOptions): MenuItem[] {
  const s = useStore.getState()
  const me = s.user
  const detail = o.communityId ? s.details[o.communityId] : undefined
  const members = o.communityId ? s.members[o.communityId] : undefined
  const member = members?.find((m) => m.user.id === o.userId)
  const myMember = members?.find((m) => m.user.id === me?.id)
  const isSelf = o.userId === me?.id
  const user =
    member?.user ??
    s.profiles[o.userId] ??
    Object.values(s.dms)
      .flatMap((d) => d.recipients)
      .find((u) => u.id === o.userId)
  const name = member ? memberName(member) : displayName(user ?? null)
  const iAmOwner = !!detail && detail.community.owner_id === me?.id
  const ruleInput = {
    myPerms: detail?.permissions ?? 0,
    iAmOwner,
    myRoleIds: iAmOwner ? [] : myMember?.role_ids,
    isSelf,
    targetIsOwner: !!detail && detail.community.owner_id === o.userId,
    targetRoleIds: detail ? member?.role_ids : undefined,
    roles: detail?.roles ?? [],
  }
  const rules = memberRules(ruleInput)
  const mentionIn = mentionTarget(o.userId)
  const audio = useVoice.getState().userAudio[o.userId] ?? DEFAULT_USER_AUDIO
  const showVoice = !!o.voice && !isSelf

  const items: MenuItem[] = [
    {
      type: 'item',
      id: 'profile',
      label: 'Profil',
      icon: <UserRound size={ICON} />,
      onSelect: () => s.openProfile(o.userId, o.communityId, anchorOf(o.target)),
    },
    {
      type: 'item',
      id: 'mention',
      label: 'Erwähnen',
      icon: <AtSign size={ICON} />,
      hidden: !mentionIn || !user,
      onSelect: () => mentionIn && user && s.insertIntoComposer(mentionIn, `@${user.username} `),
    },
    {
      type: 'item',
      id: 'message',
      label: 'Nachricht senden',
      icon: <MessageCircle size={ICON} />,
      hidden: !rules.message || useFriends.getState().relationships[o.userId]?.type === 'blocked',
      onSelect: () => runAction(() => s.openDm([o.userId])),
    },
    {
      type: 'item',
      id: 'call',
      label: 'Anrufen',
      icon: <Phone size={ICON} />,
      hidden:
        !rules.message || !s.instance?.voice || useFriends.getState().relationships[o.userId]?.type === 'blocked',
      onSelect: () => runAction(() => callUser(o.userId)),
    },
    { type: 'separator' },
    ...friendMenuItems(o.userId, name, isSelf),
    { type: 'separator', hidden: !showVoice },
    {
      type: 'custom',
      id: 'volume',
      focusable: true,
      hidden: !showVoice,
      render: () => <VolumeSlider userId={o.userId} />,
    },
    {
      type: 'checkbox',
      id: 'local-mute',
      label: 'Für mich stummschalten',
      checked: audio.muted,
      hidden: !showVoice,
      onToggle: (v) => useVoice.getState().setUserMuted(o.userId, v),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'nickname',
      label: isSelf ? 'Eigenen Nickname ändern' : 'Nickname ändern',
      icon: <UserPen size={ICON} />,
      hidden: !rules.nickname || !o.communityId,
      onSelect: () => o.communityId && openDialog({ kind: 'nickname', communityId: o.communityId, userId: o.userId }),
    },
    {
      type: 'item',
      id: 'roles',
      label: 'Rollen',
      hidden: !rules.roles,
      submenu: roleOptions(ruleInput).map((opt) => ({
        type: 'checkbox' as const,
        id: opt.role.id,
        label: opt.role.name,
        checked: opt.checked,
        disabled: !opt.enabled,
        title: opt.reason,
        color: roleColor(opt.role.color) ?? 'var(--text-muted)',
        onToggle: (on: boolean) =>
          runAction(async () => {
            if (!o.communityId) return
            if (on) await endpoints.addMemberRole(o.communityId, o.userId, opt.role.id)
            else await endpoints.removeMemberRole(o.communityId, o.userId, opt.role.id)
            await useStore.getState().loadMembers(o.communityId)
          }),
      })),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'kick',
      label: `„${name}“ entfernen`,
      icon: <UserX size={ICON} />,
      danger: true,
      hidden: !rules.kick,
      onSelect: () =>
        confirmAction({
          title: `„${name}“ entfernen`,
          message: `${name} wird aus „${detail?.community.name ?? 'der Community'}“ entfernt und kann nur mit einer neuen Einladung wieder beitreten.`,
          confirmLabel: 'Entfernen',
          action: async () => {
            if (!o.communityId) return
            await endpoints.removeMember(o.communityId, o.userId)
            await useStore.getState().loadMembers(o.communityId)
          },
        }),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'copy-username',
      label: 'Benutzernamen kopieren',
      icon: <Copy size={ICON} />,
      hidden: !user,
      onSelect: () => user && void copyText(user.username, 'Benutzername kopiert'),
    },
    copyIdItem(o.userId, 'Nutzer-ID kopieren'),
  ]
  return items
}

// ---------------------------------------------------------------------------
// Freundesystem
// ---------------------------------------------------------------------------

/** Freundschaft beenden (mit Rückfrage). */
export function removeFriend(userId: string, name: string) {
  confirmAction({
    title: `„${name}“ entfernen`,
    message: `Willst du ${name} wirklich aus deinen Freunden entfernen? Eure Direktnachrichten bleiben erhalten.`,
    confirmLabel: 'Freund entfernen',
    action: () => useFriends.getState().remove(userId),
  })
}

/** Nutzer blockieren (mit Rückfrage). */
export function blockUser(userId: string, name: string) {
  confirmAction({
    title: `${name} blockieren`,
    message: `${name} kann dir dann keine Direktnachrichten und keine Freundschaftsanfragen mehr schicken; eine bestehende Freundschaft endet. Nachrichten von ${name} werden ausgeblendet. ${name} erfährt nicht ausdrücklich davon.`,
    confirmLabel: 'Blockieren',
    action: () => useFriends.getState().block(userId),
  })
}

/** Einträge „Freund hinzufügen/entfernen, Anfrage annehmen/ablehnen/zurückziehen, Blockieren“. */
export function friendMenuItems(userId: string, name: string, isSelf: boolean): MenuItem[] {
  const f = useFriends.getState()
  const can = friendActions(f.relationships[userId]?.type, isSelf)
  return [
    {
      type: 'item',
      id: 'friend-add',
      label: 'Freund hinzufügen',
      icon: <UserPlus size={ICON} />,
      hidden: !can.add,
      onSelect: () => void addFriendWithFeedback(userId, name),
    },
    {
      type: 'item',
      id: 'friend-accept',
      label: 'Anfrage annehmen',
      icon: <Check size={ICON} />,
      hidden: !can.accept,
      onSelect: () => void addFriendWithFeedback(userId, name),
    },
    {
      type: 'item',
      id: 'friend-decline',
      label: 'Anfrage ablehnen',
      icon: <X size={ICON} />,
      hidden: !can.decline,
      onSelect: () => runAction(() => f.remove(userId)),
    },
    {
      type: 'item',
      id: 'friend-cancel',
      label: 'Anfrage zurückziehen',
      icon: <X size={ICON} />,
      hidden: !can.cancel,
      onSelect: () => runAction(() => f.remove(userId)),
    },
    {
      type: 'item',
      id: 'friend-remove',
      label: 'Freund entfernen',
      icon: <UserMinus size={ICON} />,
      danger: true,
      hidden: !can.remove,
      onSelect: () => removeFriend(userId, name),
    },
    {
      type: 'item',
      id: 'block',
      label: 'Blockieren',
      icon: <Ban size={ICON} />,
      danger: true,
      hidden: !can.block,
      onSelect: () => blockUser(userId, name),
    },
    {
      type: 'item',
      id: 'unblock',
      label: 'Blockade aufheben',
      icon: <UserCheck size={ICON} />,
      hidden: !can.unblock,
      onSelect: () => runAction(() => f.remove(userId)),
    },
  ]
}

/**
 * Kachel der Call-Bühne: Kachel-Aktionen (Fokussieren, Vollbild, Stream ansehen/beenden), darunter
 * das Nutzer-Menü mit lokaler Lautstärke.
 */
export function buildCallTileMenu(o: {
  tile: CallTile
  isMe: boolean
  target: HTMLElement
  onFullscreen: () => void
}): MenuItem[] {
  const v = useVoice.getState()
  const focused = v.focus === o.tile.key
  const screen = o.tile.kind === 'screen'
  return [
    {
      type: 'item',
      id: 'focus',
      label: focused ? 'Fokus beenden' : 'Fokussieren',
      icon: focused ? <Minimize2 size={ICON} /> : <Focus size={ICON} />,
      hidden: screen && !o.tile.watching,
      onSelect: () => v.setFocus(focused ? null : o.tile.key),
    },
    {
      type: 'item',
      id: 'fullscreen',
      label: 'Vollbild',
      icon: <Maximize size={ICON} />,
      onSelect: o.onFullscreen,
    },
    {
      type: 'item',
      id: 'watch',
      label: 'Stream ansehen',
      icon: <Eye size={ICON} />,
      hidden: !screen || o.tile.watching,
      onSelect: () => {
        v.watch(o.tile.userId)
        v.setFocus(o.tile.key)
      },
    },
    {
      type: 'item',
      id: 'unwatch',
      label: 'Zuschauen beenden',
      icon: <EyeOff size={ICON} />,
      hidden: !screen || !o.tile.watching || o.isMe,
      onSelect: () => v.unwatch(o.tile.userId),
    },
    { type: 'separator' },
    ...(v.communityId
      ? buildUserMenu({ userId: o.tile.userId, communityId: v.communityId, target: o.target, voice: true })
      : []),
  ]
}

// ---------------------------------------------------------------------------
// Kanal (Text/Voice)
// ---------------------------------------------------------------------------

export function buildChannelMenu(channel: Channel): MenuItem[] {
  const s = useStore.getState()
  const detail = s.details[channel.community_id]
  if (!detail) return []
  const perms = detail.channel_permissions[channel.id] ?? detail.permissions
  const kind = channel.kind === 'voice' ? 'voice' : 'text'
  const rules = channelRules({ channelPerms: perms, communityPerms: detail.permissions, kind })
  const voice = useVoice.getState()
  const inThisVoice = voice.status !== 'idle' && voice.channelId === channel.id
  const voiceEnabled = !!s.instance?.voice
  const label = channelLabel(channel)

  return [
    {
      type: 'item',
      id: 'read',
      label: 'Als gelesen markieren',
      icon: <CheckCheck size={ICON} />,
      hidden: !rules.markRead,
      disabled: !isChannelUnread(detail, channel.id),
      onSelect: () => s.markChannelRead(channel.community_id, channel.id),
    },
    {
      type: 'item',
      id: 'join',
      label: 'Beitreten',
      icon: <PhoneCall size={ICON} />,
      hidden: kind !== 'voice' || inThisVoice || !rules.connect || !voiceEnabled,
      onSelect: () => runAction(() => voice.join(channel)),
    },
    {
      type: 'item',
      id: 'leave-voice',
      label: 'Verlassen',
      icon: <PhoneOff size={ICON} />,
      hidden: !inThisVoice,
      onSelect: () => runAction(() => voice.leave()),
    },
    { type: 'separator' },
    muteItem({ channelId: channel.id }, 'Kanal'),
    { type: 'separator' },
    {
      type: 'item',
      id: 'invite',
      label: 'Leute einladen',
      icon: <UserPlus size={ICON} />,
      hidden: !rules.invite,
      onSelect: () => openDialog({ kind: 'invite', communityId: channel.community_id }),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'edit',
      label: 'Kanal bearbeiten',
      icon: <Settings size={ICON} />,
      hidden: !rules.edit,
      onSelect: () => openDialog({ kind: 'channelSettings', communityId: channel.community_id, channelId: channel.id }),
    },
    {
      type: 'item',
      id: 'delete',
      label: 'Kanal löschen',
      icon: <Trash2 size={ICON} />,
      danger: true,
      hidden: !rules.delete,
      onSelect: () =>
        confirmAction({
          title: 'Kanal löschen',
          message:
            kind === 'text'
              ? `Willst du ${label} wirklich löschen? Alle Nachrichten und Anhänge darin werden endgültig gelöscht.`
              : `Willst du den Sprachkanal „${label}“ wirklich löschen?`,
          confirmLabel: 'Kanal löschen',
          action: () => endpoints.deleteChannel(channel.id),
        }),
    },
    { type: 'separator' },
    copyIdItem(channel.id, 'Kanal-ID kopieren'),
  ]
}

// ---------------------------------------------------------------------------
// Community (Rail-Kachel, Kopf-Dropdown)
// ---------------------------------------------------------------------------

/** Community verlassen – mit Bestätigung. */
export function leaveCommunity(communityId: string) {
  const s = useStore.getState()
  const name = s.details[communityId]?.community.name ?? 'die Community'
  confirmAction({
    title: `„${name}“ verlassen`,
    message: `Willst du „${name}“ wirklich verlassen? Du kannst nur über eine neue Einladung wieder beitreten.`,
    confirmLabel: 'Community verlassen',
    action: async () => {
      const me = useStore.getState().user
      if (!me) return
      await endpoints.removeMember(communityId, me.id)
      useStore.getState().forgetCommunity(communityId)
    },
  })
}

/**
 * Community-Menü. `variant = 'header'` ist das Dropdown am Community-Namen (Einstellungen direkt,
 * „Leute einladen“ hervorgehoben); `'rail'` das Kontextmenü der Kachel (Einstellungen als Untermenü).
 */
export function buildCommunityMenu(communityId: string, variant: 'rail' | 'header'): MenuItem[] {
  const s = useStore.getState()
  const detail = s.details[communityId]
  if (!detail) return []
  const isOwner = detail.community.owner_id === s.user?.id
  const rules = communityRules(detail.permissions, isOwner)
  const tabs = communityTabs(detail.permissions, isOwner).filter((t) => t.id !== 'danger')
  const openSettings = (tab?: CommunityTab) => openDialog({ kind: 'communitySettings', communityId, tab })

  return [
    {
      // Kopf des Dropdowns: Banner, Icon, Beschreibung (Paket J).
      type: 'custom',
      id: 'info',
      hidden: variant !== 'header',
      render: () => <CommunityInfoCard community={detail.community} />,
    },
    { type: 'separator', hidden: variant !== 'header' },
    {
      type: 'item',
      id: 'read',
      label: 'Als gelesen markieren',
      icon: <CheckCheck size={ICON} />,
      disabled: !isCommunityUnread(detail),
      onSelect: () => s.markCommunityRead(communityId),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'invite',
      label: 'Leute einladen',
      icon: <UserPlus size={ICON} />,
      hidden: !rules.invite,
      onSelect: () => openDialog({ kind: 'invite', communityId }),
    },
    { type: 'separator', hidden: variant === 'rail' },
    variant === 'header'
      ? {
          type: 'item',
          id: 'settings',
          label: 'Community-Einstellungen',
          icon: <Settings size={ICON} />,
          onSelect: () => openSettings(),
        }
      : {
          type: 'item',
          id: 'settings',
          label: 'Community-Einstellungen',
          submenu: tabs.map((t) => ({
            type: 'item' as const,
            id: `tab-${t.id}`,
            label: t.label,
            onSelect: () => openSettings(t.id),
          })),
        },
    {
      type: 'item',
      id: 'create-channel',
      label: 'Kanal erstellen',
      icon: <Plus size={ICON} />,
      hidden: !rules.createChannel,
      onSelect: () => openDialog({ kind: 'createChannel', communityId }),
    },
    { type: 'separator' },
    muteItem({ communityId }, 'Community'),
    { type: 'separator' },
    {
      type: 'item',
      id: 'leave',
      label: 'Community verlassen',
      icon: <LogOut size={ICON} />,
      danger: true,
      hidden: !rules.leave,
      onSelect: () => leaveCommunity(communityId),
    },
    {
      type: 'item',
      id: 'delete',
      label: 'Community löschen',
      icon: <Trash2 size={ICON} />,
      danger: true,
      hidden: !rules.delete,
      // Löschen verlangt die Eingabe des Namens – das passiert in der Gefahrenzone der Einstellungen.
      onSelect: () => openSettings('danger'),
    },
    { type: 'separator' },
    copyIdItem(communityId, 'Community-ID kopieren'),
  ]
}

// ---------------------------------------------------------------------------
// Direktnachrichten
// ---------------------------------------------------------------------------

export function leaveGroupDm(dm: DmChannel) {
  const title = dmTitle(dm, useStore.getState().user?.id)
  confirmAction({
    title: 'Gruppe verlassen',
    message: `Willst du „${title}“ wirklich verlassen? Du kannst nur wieder hinzukommen, wenn jemand eine neue Gruppe mit dir startet.`,
    confirmLabel: 'Gruppe verlassen',
    action: () => useStore.getState().leaveDm(dm.id),
  })
}

export function buildDmMenu(dmId: string, target: HTMLElement): MenuItem[] {
  const s = useStore.getState()
  const dm = s.dms[dmId]
  if (!dm) return []
  const other = dm.kind === 'dm' ? dm.recipients.find((u) => u.id !== s.user?.id) : undefined
  const group = dm.kind === 'group_dm'
  return [
    {
      type: 'item',
      id: 'read',
      label: 'Als gelesen markieren',
      icon: <CheckCheck size={ICON} />,
      disabled: !isDmUnread(dm),
      onSelect: () => s.markChannelRead(null, dm.id),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'profile',
      label: 'Profil',
      icon: <UserRound size={ICON} />,
      hidden: !other,
      onSelect: () => other && s.openProfile(other.id, null, anchorOf(target)),
    },
    {
      type: 'item',
      id: 'call',
      label: group ? 'Gruppenanruf starten' : 'Anrufen',
      icon: <Phone size={ICON} />,
      hidden:
        !s.instance?.voice ||
        (useVoice.getState().status !== 'idle' && useVoice.getState().channelId === dm.id) ||
        (!!other && useFriends.getState().relationships[other.id]?.type === 'blocked'),
      onSelect: () => runAction(() => startCall(dm.id)),
    },
    {
      type: 'item',
      id: 'rename',
      label: 'Gruppe bearbeiten',
      icon: <Pencil size={ICON} />,
      hidden: !group,
      onSelect: () => openDialog({ kind: 'groupDm', dmId }),
    },
    ...(other ? friendMenuItems(other.id, displayName(other), false) : []),
    { type: 'separator' },
    muteItem({ channelId: dm.id }, 'Unterhaltung'),
    { type: 'separator' },
    {
      type: 'item',
      id: 'copy-username',
      label: 'Benutzernamen kopieren',
      icon: <Copy size={ICON} />,
      hidden: !other,
      onSelect: () => other && void copyText(other.username, 'Benutzername kopiert'),
    },
    {
      type: 'item',
      id: 'leave',
      label: 'Gruppe verlassen',
      icon: <LogOut size={ICON} />,
      danger: true,
      hidden: !group,
      onSelect: () => leaveGroupDm(dm),
    },
    { type: 'separator' },
    copyIdItem(dm.id, 'Kanal-ID kopieren'),
    { ...copyIdItem(other?.id ?? '', 'Nutzer-ID kopieren'), id: 'copy-user-id', hidden: !other || !devMode() },
  ]
}

// ---------------------------------------------------------------------------
// Eigene Nutzerleiste unten links
// ---------------------------------------------------------------------------

export function buildSelfMenu(): MenuItem[] {
  const s = useStore.getState()
  const me = s.user
  if (!me) return []
  const dot = (status: PresenceStatus) => <span className={`presence-dot presence-${status}`} />
  return [
    { type: 'header', label: 'Status' },
    ...choosableStatuses.map(({ status, hint }) => ({
      type: 'radio' as const,
      id: `status-${status}`,
      label: presenceLabel[status],
      description: hint,
      checked: s.chosenStatus === status,
      icon: dot(status === 'invisible' ? 'offline' : status),
      onSelect: () => s.setChosenStatus(status),
    })),
    { type: 'separator' },
    {
      type: 'item',
      id: 'status-text',
      label: me.status_text ? 'Status-Text ändern' : 'Status-Text setzen',
      icon: <MessageCircle size={ICON} />,
      onSelect: () => openDialog({ kind: 'statusText' }),
    },
    {
      type: 'item',
      id: 'clear-status-text',
      label: 'Status-Text entfernen',
      icon: <CircleX size={ICON} />,
      hidden: !me.status_text,
      onSelect: () => runAction(async () => s.setUser(await endpoints.updateMe({ status_text: null }))),
    },
    { type: 'separator' },
    {
      type: 'item',
      id: 'edit-profile',
      label: 'Profil bearbeiten',
      icon: <UserPen size={ICON} />,
      onSelect: () => openDialog({ kind: 'settings', tab: 'profile' }),
    },
    {
      type: 'item',
      id: 'settings',
      label: 'Einstellungen',
      hint: 'Strg+,',
      icon: <Settings size={ICON} />,
      onSelect: () => openDialog({ kind: 'settings' }),
    },
    { type: 'separator' },
    copyIdItem(me.id, 'Nutzer-ID kopieren'),
    {
      type: 'item',
      id: 'logout',
      label: 'Abmelden',
      icon: <LogOut size={ICON} />,
      danger: true,
      onSelect: () =>
        confirmAction({
          title: 'Abmelden',
          message: 'Willst du dich auf diesem Gerät wirklich abmelden?',
          confirmLabel: 'Abmelden',
          action: () => useStore.getState().logout(),
        }),
    },
  ]
}
