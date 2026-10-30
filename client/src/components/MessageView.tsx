import { Fragment, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent, KeyboardEvent } from 'react'
import { isDesktopBundle } from '../lib/api'
import type { Attachment, Member, Message, MessageChannel, UserPublic } from '../lib/api'
import { errorText } from '../lib/errors'
import { Markdown } from '../lib/markdown'
import type { MarkdownOptions } from '../lib/markdown'
import { plainText } from '../lib/markdownParser'
import { mentionQuery, mentionResolver, mentionSuggestions } from '../lib/mentions'
import type { MentionSuggestion } from '../lib/mentions'
import { buildMessageMenu, buildUserMenu, deleteMessage, togglePin } from '../lib/contextMenus'
import { useContextMenu } from '../lib/menuStore'
import { isChannelMuted, setChannelMuted, useMutesVersion } from '../lib/notify'
import { Permission, has } from '../lib/permissions'
import { isNewer } from '../lib/unread'
import { blockedLabel, blockedRunStart, blockedRuns } from '../lib/friends'
import { useFriends } from '../lib/friendsStore'
import { channelLabel, displayName, lastMessageOf, mentionsUser, readStateOf, senderName, useMemberName, useStore } from '../store'
import { useUploads } from '../lib/uploads'
import { useFileDrop, usePasteFiles } from '../lib/fileInput'
import { AttachButton, AttachmentList, DropOverlay, Lightbox, PendingUploads } from './Attachments'
import { BotTag } from './BotTag'
import { EmbedList } from './Embeds'
import { PinnedMessages } from './PinnedMessages'
import { DrawerButton } from './MobileNav'
import { EmojiPicker, ReactionBar } from './Reactions'
import { EmojiButton } from './EmojiButton'
import { useComposerEmojis, useEmojiAutocomplete } from './EmojiAutocomplete'
import { Avatar } from './Presence'
import { TypingIndicator } from './TypingIndicator'
import { AutoGrowTextarea } from './AutoGrowTextarea'
import { useAutoGrow } from '../lib/autoGrow'
import { SystemMessage } from './customize/CommunityView'
import { AutoModAlert } from './AutoModAlert'
import { CallMissedMessage, DmCallButtons, DmCallSection } from './DmCall'
import { DmAddPeopleButton, RecipientAddMessage } from './DirectMessages'
import {
  AtSign,
  Bell,
  BellOff,
  Hash,
  Pencil,
  Pin,
  PinOff,
  Reply,
  Search,
  SmilePlus,
  Trash2,
  Users,
  X,
  EyeOff,
} from 'lucide-react'

/** Aufeinanderfolgende Nachrichten desselben Autors innerhalb dieses Fensters werden gruppiert. */
const GROUP_WINDOW_MS = 7 * 60 * 1000
const MAX_LENGTH = 4000
/** So lange bleibt eine angesprungene Nachricht hervorgehoben. */
const HIGHLIGHT_MS = 2000

const timeFmt = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' })
const dateTimeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

function ChannelIcon({ kind, size }: { kind: MessageChannel['kind']; size: number }) {
  if (kind === 'dm') return <AtSign size={size} />
  if (kind === 'group_dm') return <Users size={size} />
  return <Hash size={size} />
}

function formatTimestamp(iso: string): string {
  const d = new Date(iso)
  const today = new Date()
  if (d.toDateString() === today.toDateString()) return `Heute um ${timeFmt.format(d)}`
  return dateTimeFmt.format(d)
}

export function MessageView({
  channel,
  permissions,
  readOnlyHint,
}: {
  channel: MessageChannel
  permissions: number
  /** Platzhalter im Eingabefeld, wenn man nicht schreiben darf (Standard: fehlendes Recht). */
  readOnlyHint?: string
}) {
  const state = useStore((s) => s.messages[channel.id])
  const loadMessages = useStore((s) => s.loadMessages)
  const user = useStore((s) => s.user)
  const communityId = channel.community_id
  const communityMembers = useStore((s) => (communityId ? s.members[communityId] : undefined))
  const recipients = useStore((s) => (communityId ? undefined : s.dms[channel.id]?.recipients))
  // In DMs sind die Teilnehmer die „Mitglieder“ (Erwähnungen, Autovervollständigung).
  const members = useMemo(() => communityMembers ?? recipients?.map(asMember), [communityMembers, recipients])
  const loadMembers = useStore((s) => s.loadMembers)
  const [error, setError] = useState<string | null>(null)
  const [replyTo, setReplyTo] = useState<Message | null>(null)
  const [highlight, setHighlight] = useState<string | null>(null)

  const scrollRef = useRef<HTMLDivElement>(null)
  const atBottom = useRef(true)
  /** Scrollhöhe vor dem Nachladen älterer Nachrichten, um die Position zu halten. */
  const prevHeight = useRef<number | null>(null)
  /** Beim Nachladen neuerer Nachrichten (nach einem Sprung) nicht ans Ende springen. */
  const keepPosition = useRef(false)
  const loadNewer = useStore((s) => s.loadNewer)

  // Mitgliederliste für Nicknames der Autoren und Erwähnungen (auch wenn die Seitenleiste ausgeblendet ist).
  useEffect(() => {
    if (communityId && !useStore.getState().members[communityId]) loadMembers(communityId).catch(() => {})
  }, [communityId, loadMembers])

  useEffect(() => {
    // Komponente wird pro Kanal neu gemountet (key), daher kein Zurücksetzen nötig.
    if (!useStore.getState().messages[channel.id]) {
      loadMessages(channel.id).catch((e) => setError(errorText(e)))
    }
  }, [channel.id, loadMessages])

  const markdown = useMemo<MarkdownOptions>(
    () => ({ resolveMention: mentionResolver(members, user?.id), jumbo: true }),
    [members, user?.id],
  )

  const items = state?.items
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (!el) return
    if (prevHeight.current !== null) {
      el.scrollTop += el.scrollHeight - prevHeight.current
      prevHeight.current = null
    } else if (keepPosition.current) {
      keepPosition.current = false
    } else if (atBottom.current) {
      el.scrollTop = el.scrollHeight
    }
  }, [items])

  const onScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
    const s = useStore.getState().messages[channel.id]
    if (el.scrollTop < 120 && s && s.hasMore && !s.loading && s.items.length > 0) {
      prevHeight.current = el.scrollHeight
      loadMessages(channel.id, true).catch((e) => {
        prevHeight.current = null
        setError(errorText(e))
      })
    }
    // Nach einem Sprung aus der Suche: neuere Nachrichten nachladen, sobald man unten ankommt.
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120
    if (nearBottom && s?.hasNewer && !s.loading) {
      keepPosition.current = true
      loadNewer(channel.id).catch((e) => {
        keepPosition.current = false
        setError(errorText(e))
      })
    }
  }, [channel.id, loadMessages, loadNewer])

  // --- Sprung zu einer Nachricht (Suche): hinscrollen und kurz hervorheben ---
  const jumpTarget = useStore((s) => (s.jumpTarget?.channelId === channel.id ? s.jumpTarget.messageId : null))
  const clearJumpTarget = useStore((s) => s.clearJumpTarget)
  useEffect(() => {
    if (!jumpTarget || !state?.items.some((m) => m.id === jumpTarget)) return
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(jumpTarget)}"]`)
    if (!el) return
    el.scrollIntoView({ block: 'center' })
    atBottom.current = false
    el.classList.add('highlighted')
    window.setTimeout(() => el.classList.remove('highlighted'), HIGHLIGHT_MS)
    clearJumpTarget()
  }, [jumpTarget, state?.items, clearJumpTarget])

  /** Zur Nachricht scrollen und kurz hervorheben – nur wenn sie geladen ist. */
  const jumpTo = useCallback((id: string) => {
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(id)}"]`)
    if (!el) {
      setError('Die Nachricht ist nicht geladen – scrolle weiter nach oben.')
      return
    }
    setError(null)
    el.scrollIntoView({ block: 'center', behavior: 'smooth' })
    setHighlight(id)
  }, [])

  useEffect(() => {
    if (!highlight) return
    const t = window.setTimeout(() => setHighlight(null), HIGHLIGHT_MS)
    return () => window.clearTimeout(t)
  }, [highlight])

  // Antwort verwerfen, wenn das Original inzwischen gelöscht wurde.
  const activeReply = replyTo && items?.some((m) => m.id === replyTo.id) ? replyTo : null

  // --- Ungelesen: Lesestand setzen, solange man hinsieht; sonst „Neu“-Trennlinie merken ---
  const markRead = useStore((s) => s.markRead)
  /** Nachrichten nach dieser ID gelten als neu (Trennlinie); `null` = keine Trennlinie. */
  const [newSince, setNewSince] = useState<string | null>(() => {
    const s = useStore.getState()
    const read = readStateOf(s, channel.community_id, channel.id)
    const last = lastMessageOf(s, channel.community_id, channel.id)
    return read && last && isNewer(last, read) ? read : null
  })

  const checkRead = useCallback(() => {
    const s = useStore.getState()
    const list = s.messages[channel.id]?.items
    const last = list?.[list.length - 1]
    const read = readStateOf(s, channel.community_id, channel.id)
    if (!last || !isNewer(last.id, read)) return
    const el = scrollRef.current
    const bottom = !el || el.scrollHeight - el.scrollTop - el.clientHeight < 40
    if (document.hasFocus() && !document.hidden && bottom) {
      markRead(channel.community_id, channel.id, last.id)
    } else if (last.author?.id !== s.user?.id) {
      setNewSince((prev) => prev ?? read ?? null)
    }
  }, [channel.community_id, channel.id, markRead])

  // Nach jeder Änderung der Nachrichtenliste prüfen – im nächsten Frame, also nach dem Scrollen ans Ende.
  useEffect(() => {
    const frame = requestAnimationFrame(checkRead)
    const unsubscribe = useStore.subscribe((s, prev) => {
      if (s.messages[channel.id] !== prev.messages[channel.id]) requestAnimationFrame(checkRead)
    })
    return () => {
      cancelAnimationFrame(frame)
      unsubscribe()
    }
  }, [channel.id, checkRead])

  useEffect(() => {
    const el = scrollRef.current
    window.addEventListener('focus', checkRead)
    document.addEventListener('visibilitychange', checkRead)
    el?.addEventListener('scroll', checkRead, { passive: true })
    return () => {
      window.removeEventListener('focus', checkRead)
      document.removeEventListener('visibilitychange', checkRead)
      el?.removeEventListener('scroll', checkRead)
    }
  }, [checkRead])

  const newIndex = newSince && items ? items.findIndex((m) => m.id > newSince && m.author?.id !== user?.id) : -1
  useMutesVersion()
  const muted = isChannelMuted(channel.id)

  function toggleMuted() {
    setChannelMuted(channel.id, !muted)
  }

  // Nachrichten blockierter Nutzer: zusammenhängende Läufe als Platzhalter mit „Anzeigen“.
  const relationships = useFriends((s) => s.relationships)
  const blockedRunMap = useMemo(
    () =>
      blockedRuns(items ?? [], (id) => id !== user?.id && relationships[id]?.type === 'blocked'),
    [items, relationships, user?.id],
  )
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(() => new Set())
  function toggleRevealed(id: string) {
    setRevealed((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const canSend = has(permissions, Permission.SEND_MESSAGES)
  const canManage = has(permissions, Permission.MANAGE_MESSAGES)
  const canAttach = canSend && has(permissions, Permission.ATTACH_FILES)
  const canReact = has(permissions, Permission.ADD_REACTIONS)
  const isDm = channel.community_id === null
  // `@everyone` gibt es in DMs nicht; anpinnen dürfen dort alle Teilnehmer.
  const canMentionEveryone = !isDm && has(permissions, Permission.MENTION_EVERYONE)
  const canPin = canManage || isDm
  const drop = useFileDrop(channel.id, canAttach)

  return (
    <section className="messages-pane" {...drop.props}>
      {drop.dragging && <DropOverlay target={channelLabel(channel)} />}
      <header className="pane-header">
        <DrawerButton />
        <span className="channel-icon" aria-hidden="true">
          <ChannelIcon kind={channel.kind} size={20} />
        </span>
        <strong className="truncate">{channel.name}</strong>
        {channel.topic && (
          <span className="pane-topic truncate" title={channel.topic}>
            {channel.topic}
          </span>
        )}
        <span className="pane-actions">
          {isDm && <DmCallButtons channelId={channel.id} />}
          {isDm && <DmAddPeopleButton channelId={channel.id} />}
          <PinnedMessages channel={channel} canManage={canPin} markdown={markdown} onJump={jumpTo} />
          {isDesktopBundle && (
            <button
              type="button"
              className="icon-button"
              title={muted ? 'Benachrichtigungen einschalten' : 'Kanal stummschalten'}
              aria-pressed={muted}
              onClick={toggleMuted}
            >
              {muted ? <BellOff size={18} /> : <Bell size={18} />}
            </button>
          )}
          <SearchField communityId={channel.community_id} />
        </span>
      </header>
      {isDm && <DmCallSection channelId={channel.id} />}

      <div className="messages" ref={scrollRef} onScroll={onScroll}>
        {state && !state.hasMore && !state.loading && (
          <div className="channel-start">
            {isDm ? (
              <>
                <h2>{channel.name}</h2>
                <p className="muted">
                  Das ist der Anfang eurer {channel.kind === 'group_dm' ? 'Gruppenunterhaltung' : 'Unterhaltung'}.
                </p>
              </>
            ) : (
              <>
                <h2>Willkommen in #{channel.name}</h2>
                <p className="muted">Das ist der Anfang dieses Kanals.</p>
              </>
            )}
          </div>
        )}
        {state?.loading && <p className="muted center small">Lade Nachrichten …</p>}
        {error && <p className="error center">{error}</p>}
        {items?.map((m, i) => {
          const runStart = blockedRunStart(blockedRunMap, i)
          const runId = runStart === null ? null : items[runStart].id
          const hidden = runId !== null && !revealed.has(runId)
          if (hidden && runStart !== i) return null
          return (
          <Fragment key={m.id}>
            {i === newIndex && (
              <div className="new-divider" role="separator">
                <span>Neu</span>
              </div>
            )}
            {runId !== null && runStart === i && (
              <div className="blocked-messages">
                <EyeOff size={14} aria-hidden="true" />
                <span>{blockedLabel(blockedRunMap.get(i) ?? 1)}</span>
                <button type="button" className="link-button" onClick={() => toggleRevealed(runId)}>
                  {hidden ? 'Anzeigen' : 'Ausblenden'}
                </button>
              </div>
            )}
            {hidden ? null : m.kind === 'member_join' ? (
              <SystemMessage message={m} canDelete={canManage || m.author?.id === user?.id} markdown={markdown} />
            ) : m.kind === 'recipient_add' ? (
              <RecipientAddMessage message={m} />
            ) : m.kind === 'call_missed' ? (
              <CallMissedMessage message={m} />
            ) : m.kind === 'automod_alert' ? (
              <AutoModAlert message={m} canDelete={canManage} />
            ) : (
              <MessageItem
                message={m}
                grouped={i !== newIndex && isGrouped(items[i - 1], m)}
                own={m.author?.id === user?.id}
                mentioned={mentionsUser(m, user?.id)}
                highlighted={highlight === m.id}
                canDelete={canManage || m.author?.id === user?.id}
                canReply={canSend}
                canReact={canReact}
                canPin={canPin}
                permissions={isDm && canSend ? permissions | Permission.ADD_REACTIONS : permissions}
                markdown={markdown}
                onReply={setReplyTo}
                onJump={jumpTo}
              />
            )}
          </Fragment>
          )
        })}
      </div>

      <Composer
        channel={channel}
        disabled={!canSend}
        disabledHint={readOnlyHint}
        canAttach={canAttach}
        canMentionEveryone={canMentionEveryone}
        members={members}
        replyTo={activeReply}
        onCancelReply={() => setReplyTo(null)}
        onSent={() => (atBottom.current = true)}
      />
      <TypingIndicator channel={channel} />
    </section>
  )
}

/** Suchfeld im Kanal-Header: Enter öffnet die Ergebnisliste (Community bzw. alle eigenen DMs). */
function SearchField({ communityId }: { communityId: string | null }) {
  const openSearch = useStore((s) => s.openSearch)
  const [query, setQuery] = useState('')
  // Mobil: nur ein Lupen-Knopf; erst beim Tippen darauf legt sich das Feld über die Kopfzeile.
  const [expanded, setExpanded] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (expanded) inputRef.current?.focus()
  }, [expanded])

  function submit(e: FormEvent) {
    e.preventDefault()
    if (!query.trim()) return
    openSearch({ communityId, channelId: null, query: query.trim() })
    setExpanded(false)
  }

  return (
    <>
      <button
        type="button"
        className="icon-button pane-search-toggle mobile-only"
        aria-label={communityId ? 'Nachrichten suchen' : 'Direktnachrichten durchsuchen'}
        title="Suchen"
        onClick={() => setExpanded(true)}
      >
        <Search size={18} />
      </button>
      <form className={`pane-search${expanded ? ' expanded' : ''}`} role="search" onSubmit={submit}>
        <Search size={15} aria-hidden="true" />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={communityId ? 'Suchen' : 'DMs durchsuchen'}
          aria-label={communityId ? 'Nachrichten in dieser Community suchen' : 'Direktnachrichten durchsuchen'}
          maxLength={200}
          enterKeyHint="search"
        />
        <button
          type="button"
          className="icon-button pane-search-cancel mobile-only"
          aria-label="Suche schließen"
          onClick={() => setExpanded(false)}
        >
          <X size={18} />
        </button>
      </form>
    </>
  )
}

/** DM-Teilnehmer in der Form der Mitgliederliste (ohne Spitznamen und Rollen). */
function asMember(user: UserPublic): Member {
  return { user, nickname: null, role_ids: [], joined_at: '' }
}

/** Gleicher Absender: derselbe Nutzer oder derselbe Webhook unter demselben Namen. */
function sameSender(a: Message, b: Message): boolean {
  if (a.webhook || b.webhook) {
    return !!a.webhook && !!b.webhook && !!a.webhook_id && a.webhook_id === b.webhook_id && a.webhook.name === b.webhook.name
  }
  return !!a.author && !!b.author && a.author.id === b.author.id
}

function isGrouped(prev: Message | undefined, m: Message): boolean {
  if (!prev || !sameSender(prev, m) || m.reply_to_id) return false
  // Nach Systemnachrichten (z. B. Beitritt) beginnt immer ein neuer Block.
  if (prev.kind && prev.kind !== 'default') return false
  return new Date(m.created_at).getTime() - new Date(prev.created_at).getTime() < GROUP_WINDOW_MS
}

/** Vorschau der Nachricht, auf die geantwortet wird; Klick springt zum Original. */
function ReplyPreview({ message, onJump }: { message: Message; onJump: (id: string) => void }) {
  const ref = message.referenced_message
  if (!ref) {
    return (
      <div className="reply-preview deleted">
        <Reply size={14} className="reply-icon" />
        <span>Ursprüngliche Nachricht gelöscht</span>
      </div>
    )
  }
  const text = ref.content ? plainText(ref.content) : ref.has_attachments ? 'Anhang' : ''
  return (
    <button type="button" className="reply-preview" onClick={() => onJump(ref.id)} title="Zur Nachricht springen">
      <Reply size={14} className="reply-icon" />
      <span className="reply-author">{ref.webhook_name ?? displayName(ref.author)}</span>
      <span className="reply-text truncate">{text}</span>
    </button>
  )
}

/**
 * Eine Nachricht. `memo`: Beim Eintreffen einer neuen Nachricht, Tippen usw. rendert die Liste neu – die
 * übrigen Nachrichten bleiben unverändert, solange ihre Props gleich sind (Nachrichtenobjekte werden im
 * Store nur bei Änderungen ersetzt, Callbacks sind stabil).
 */
const MessageItem = memo(function MessageItem({
  message,
  grouped,
  own,
  mentioned,
  highlighted,
  canDelete,
  canReply,
  canReact,
  canPin,
  permissions,
  markdown,
  onReply,
  onJump,
}: {
  message: Message
  grouped: boolean
  own: boolean
  mentioned: boolean
  highlighted: boolean
  canDelete: boolean
  canReply: boolean
  canReact: boolean
  canPin: boolean
  /** Effektive Rechte im Kanal (für das Kontextmenü). */
  permissions: number
  markdown: MarkdownOptions
  onReply: (message: Message) => void
  onJump: (id: string) => void
}) {
  const toggleReaction = useStore((s) => s.toggleReaction)
  const openProfile = useStore((s) => s.openProfile)
  const memberName = useMemberName(message.community_id, message.author)
  const webhook = message.webhook ?? null
  // Webhooks zeigen ihren eigenen Namen und Avatar (nie eine fremde Bild-URL).
  const authorName = webhook ? webhook.name : memberName
  const avatarUser = webhook ? { id: message.webhook_id ?? webhook.name, avatar_url: webhook.avatar_url } : message.author
  const [editing, setEditing] = useState(false)
  const [picker, setPicker] = useState(false)
  const [lightbox, setLightbox] = useState<Attachment | null>(null)
  const [error, setError] = useState<string | null>(null)
  const closePicker = useCallback(() => setPicker(false), [])
  const reply = () => onReply(message)

  function startEdit() {
    setEditing(true)
  }

  const menu = useContextMenu(
    (ctx) =>
      buildMessageMenu({
        message,
        perms: permissions,
        target: ctx.target,
        onReply: reply,
        onEdit: startEdit,
        onMoreReactions: () => setPicker(true),
        onOpenAttachment: setLightbox,
      }),
    'Nachricht',
  )
  const authorMenu = useContextMenu(
    (ctx) =>
      message.author
        ? buildUserMenu({ userId: message.author.id, communityId: message.community_id, target: ctx.target })
        : [],
    authorName,
  )

  function react(emoji: string) {
    toggleReaction(message, emoji).then(
      () => setError(null),
      (e) => setError(errorText(e)),
    )
  }

  const options = useMemo(() => ({ ...markdown, everyone: message.mention_everyone }), [markdown, message.mention_everyone])
  const classes = ['message']
  if (grouped) classes.push('grouped')
  if (mentioned) classes.push('mentioned')
  if (highlighted) classes.push('highlighted')
  if (picker) classes.push('picking')

  return (
    <article className={classes.join(' ')} data-message-id={message.id} {...menu}>
      {message.reply_to_id && <ReplyPreview message={message} onJump={onJump} />}
      {!grouped && (
        <button
          type="button"
          className="message-avatar"
          tabIndex={-1}
          aria-hidden="true"
          disabled={!message.author}
          data-menu-anchor
          onContextMenu={authorMenu.onContextMenu}
          onClick={(e) =>
            message.author && openProfile(message.author.id, message.community_id, e.currentTarget.getBoundingClientRect())
          }
        >
          <Avatar user={avatarUser} name={authorName} size={40} />
        </button>
      )}
      {!grouped && (
        <header className="message-header">
          {webhook ? (
            <>
              <span className="message-author webhook">{authorName}</span>
              <BotTag kind="webhook" />
            </>
          ) : message.author ? (
            <button
              type="button"
              className="message-author"
              data-menu-anchor
              {...authorMenu}
              onClick={(e) =>
                openProfile(message.author!.id, message.community_id, e.currentTarget.getBoundingClientRect())
              }
            >
              {authorName}
            </button>
          ) : (
            <span className="message-author deleted">{authorName}</span>
          )}
          {message.author?.bot && <BotTag />}
          <time className="message-time" dateTime={message.created_at}>
            {formatTimestamp(message.created_at)}
          </time>
          {message.pinned_at && (
            <span className="message-pinned" title="Angepinnt">
              <Pin size={12} />
            </span>
          )}
        </header>
      )}
      {editing ? (
        <MessageEditor
          message={message}
          onSaved={() => setError(null)}
          onError={setError}
          onClose={() => setEditing(false)}
        />
      ) : (message.content || message.edited_at) && (
        // Markdown wird als React-Elemente gerendert (kein HTML aus Nutzertext).
        <div className="message-content">
          <Markdown content={message.content} options={options} />
          {message.edited_at && (
            <span className="message-edited" title={formatTimestamp(message.edited_at)}>
              {' '}
              (bearbeitet)
            </span>
          )}
        </div>
      )}
      <EmbedList embeds={message.embeds} markdown={markdown} />
      <AttachmentList attachments={message.attachments} />
      <ReactionBar message={message} canReact={canReact} />
      {error && <p className="error small">{error}</p>}
      {!editing && (
        <div className="message-actions">
          {canReact && (
            <span className="reaction-add-wrap">
              <button type="button" className="icon-button small" title="Reaktion hinzufügen" onClick={() => setPicker(true)}>
                <SmilePlus size={16} />
              </button>
              {picker && <EmojiPicker onPick={react} onClose={closePicker} />}
            </span>
          )}
          {canReply && (
            <button type="button" className="icon-button small" title="Antworten" onClick={reply}>
              <Reply size={16} />
            </button>
          )}
          {canPin && (
            <button
              type="button"
              className="icon-button small"
              title={message.pinned_at ? 'Lösen' : 'Anpinnen'}
              onClick={() => togglePin(message)}
            >
              {message.pinned_at ? <PinOff size={16} /> : <Pin size={16} />}
            </button>
          )}
          {own && (
            <button type="button" className="icon-button small" title="Bearbeiten" onClick={startEdit}>
              <Pencil size={16} />
            </button>
          )}
          {canDelete && (
            <button
              type="button"
              className="icon-button small danger"
              title="Löschen (Umschalt: ohne Nachfrage)"
              onClick={(e) => deleteMessage(message, e.shiftKey)}
            >
              <Trash2 size={16} />
            </button>
          )}
        </div>
      )}
      {lightbox && <Lightbox attachment={lightbox} onClose={() => setLightbox(null)} />}
    </article>
  )
})

/**
 * Bearbeiten einer eigenen Nachricht. Eigene Komponente, damit die Emoji-Daten (`useComposerEmojis`)
 * nur im Bearbeiten-Modus abonniert werden – nicht von jeder Nachricht der Liste.
 */
function MessageEditor({
  message,
  onSaved,
  onError,
  onClose,
}: {
  message: Message
  onSaved: () => void
  onError: (error: string) => void
  onClose: () => void
}) {
  const editMessage = useStore((s) => s.editMessage)
  const composerEmojis = useComposerEmojis(message.community_id)
  const [draft, setDraft] = useState(() => composerEmojis.decode(message.content))

  async function save() {
    const content = composerEmojis.encode(draft.trim())
    if (!content || content === message.content) {
      onClose()
      return
    }
    try {
      await editMessage(message.id, content)
      onSaved()
      onClose()
    } catch (e) {
      onError(errorText(e))
    }
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      onClose()
    }
  }

  return (
    <div className="message-edit">
      <div className="emoji-field emoji-field-area">
        <AutoGrowTextarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKey}
          maxLength={MAX_LENGTH}
          autoFocus
        />
        <EmojiButton className="emoji-field-button" custom customText={composerEmojis.insertText} size={16} />
      </div>
      <span className="muted small">Enter zum Speichern · Esc zum Abbrechen</span>
    </div>
  )
}

function Composer({
  channel,
  disabled,
  disabledHint,
  canAttach,
  canMentionEveryone,
  members,
  replyTo,
  onCancelReply,
  onSent,
}: {
  channel: MessageChannel
  disabled: boolean
  disabledHint?: string
  canAttach: boolean
  canMentionEveryone: boolean
  members: Member[] | undefined
  replyTo: Message | null
  onCancelReply: () => void
  onSent: () => void
}) {
  const sendMessage = useStore((s) => s.sendMessage)
  const uploads = useUploads((s) => s.byChannel[channel.id])
  const clearSent = useUploads((s) => s.clearSent)
  const onPaste = usePasteFiles(channel.id, canAttach && !disabled)
  const attachmentIds = (uploads ?? []).flatMap((u) => (u.attachment ? [u.attachment.id] : []))
  const uploading = (uploads ?? []).some((u) => u.status === 'uploading')
  const sendTyping = useStore((s) => s.sendTyping)
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const draft = drafts[channel.id] ?? ''
  const textRef = useRef<HTMLTextAreaElement>(null)
  // Höhe folgt dem Inhalt (auch umbrochenen Zeilen); Scrollleiste erst ab max-height.
  useAutoGrow(textRef, draft)

  // --- @-Autovervollständigung ---
  const [mention, setMention] = useState<{ start: number; query: string } | null>(null)
  const [selected, setSelected] = useState(0)
  const suggestions = useMemo(
    () => (mention ? mentionSuggestions(members, mention.query, canMentionEveryone) : []),
    [mention, members, canMentionEveryone],
  )

  // --- Emojis: `:shortcode:`-Vervollständigung, Custom-Emotes als `:name:` im Feld (beim Senden zu Tokens) ---
  const composerEmojis = useComposerEmojis(channel.community_id)
  const emoji = useEmojiAutocomplete({
    textRef,
    value: draft,
    setValue: setDraft,
    customs: composerEmojis.customs,
    insertText: composerEmojis.insertText,
  })

  // Beim Antworten direkt ins Textfeld springen.
  useEffect(() => {
    if (replyTo) textRef.current?.focus()
  }, [replyTo])

  // „Erwähnen“ aus einem Kontextmenü: Text ans Ende anhängen und Fokus ins Feld.
  useEffect(
    () =>
      useStore.subscribe((s) => {
        const insert = s.composerInsert
        if (!insert || insert.channelId !== channel.id) return
        s.clearComposerInsert()
        setDrafts((d) => {
          const current = d[channel.id] ?? ''
          const sep = current && !/\s$/.test(current) ? ' ' : ''
          return { ...d, [channel.id]: current + sep + insert.text }
        })
        requestAnimationFrame(() => {
          const el = textRef.current
          if (!el) return
          el.focus()
          el.setSelectionRange(el.value.length, el.value.length)
        })
      }),
    [channel.id],
  )

  function setDraft(value: string) {
    setDrafts((d) => ({ ...d, [channel.id]: value }))
  }

  function updateMention(value: string, caret: number) {
    const q = mentionQuery(value, caret)
    setMention(q)
    if (q?.query !== mention?.query) setSelected(0)
  }

  function insertMention(s: MentionSuggestion) {
    if (!mention) return
    const el = textRef.current
    const caret = el?.selectionStart ?? draft.length
    const text = `@${s.insert} `
    const next = draft.slice(0, mention.start) + text + draft.slice(caret)
    setDraft(next)
    setMention(null)
    const pos = mention.start + text.length
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(pos, pos)
    })
  }

  async function send() {
    const content = draft.trim()
    if ((!content && attachmentIds.length === 0) || sending) return
    if (uploading) {
      setError('Bitte warte, bis alle Uploads fertig sind.')
      return
    }
    setSending(true)
    setError(null)
    try {
      onSent()
      await sendMessage(channel.id, composerEmojis.encode(content), attachmentIds, replyTo?.id)
      if (attachmentIds.length > 0) clearSent(channel.id)
      setDraft('')
      setMention(null)
      onCancelReply()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSending(false)
    }
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (emoji.onKeyDown(e)) return
    if (suggestions.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const delta = e.key === 'ArrowDown' ? 1 : -1
        setSelected((i) => (i + delta + suggestions.length) % suggestions.length)
        return
      }
      if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
        e.preventDefault()
        insertMention(suggestions[Math.min(selected, suggestions.length - 1)])
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setMention(null)
        return
      }
    }
    if (e.key === 'Escape' && replyTo) {
      e.preventDefault()
      onCancelReply()
      return
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      void send()
    }
  }

  return (
    <div className="composer">
      {error && <p className="error small">{error}</p>}
      {canAttach && <PendingUploads channelId={channel.id} />}
      {replyTo && (
        <div className="reply-bar">
          <Reply size={14} />
          <span className="truncate">
            Antwort an <strong>{senderName(replyTo)}</strong>
          </span>
          <button type="button" className="icon-button small" title="Antwort abbrechen" onClick={onCancelReply}>
            <X size={14} />
          </button>
        </div>
      )}
      <div className="composer-row">
        {canAttach && <AttachButton channelId={channel.id} disabled={disabled} />}
        <div className="composer-input">
          {suggestions.length > 0 && (
            <ul className="mention-suggestions" role="listbox" aria-label="Erwähnen">
              {suggestions.map((s, i) => (
                <li key={s.insert} role="option" aria-selected={i === selected}>
                  <button
                    type="button"
                    className={i === selected ? 'active' : ''}
                    // mousedown statt click, damit das Textfeld den Fokus behält.
                    onMouseDown={(e) => {
                      e.preventDefault()
                      insertMention(s)
                    }}
                    onMouseEnter={() => setSelected(i)}
                  >
                    <strong>{s.insert === 'everyone' ? '@everyone' : s.label}</strong>
                    <span className="muted small">{s.insert === 'everyone' ? s.detail : `@${s.detail}`}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {emoji.list}
          <textarea
            ref={textRef}
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value)
              updateMention(e.target.value, e.target.selectionStart)
              emoji.update(e.target.value, e.target.selectionStart)
              // Tipp-Anzeige für andere (im Store gedrosselt).
              if (e.target.value.trim()) sendTyping(channel.id)
            }}
            onSelect={(e) => {
              updateMention(e.currentTarget.value, e.currentTarget.selectionStart)
              emoji.update(e.currentTarget.value, e.currentTarget.selectionStart)
            }}
            onBlur={() => {
              setMention(null)
              emoji.close()
            }}
            onKeyDown={onKey}
            onPaste={onPaste}
            placeholder={
              disabled
                ? (disabledHint ?? 'Du darfst in diesem Kanal nicht schreiben.')
                : `Nachricht an ${channelLabel(channel)}`
            }
            disabled={disabled}
            maxLength={MAX_LENGTH}
            rows={1}
          />
          <EmojiButton
            target={textRef}
            custom
            customText={composerEmojis.insertText}
            disabled={disabled}
            className="composer-emoji"
            size={20}
          />
        </div>
      </div>
    </div>
  )
}
