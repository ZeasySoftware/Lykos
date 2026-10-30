import { create } from 'zustand'
import { ApiError, endpoints, isMfaChallenge, setToken } from './lib/api'
import type {
  AuthResponse,
  Community,
  CommunityDetail,
  DmChannel,
  InstanceInfo,
  MfaChallenge,
  MfaMethod,
  Member,
  Message,
  Role,
  User,
  UserProfile,
  UserPublic,
} from './lib/api'
import type { VoiceState } from './lib/api'
import { Gateway } from './lib/gateway'
import type { GatewayEvent, GatewayStatus } from './lib/gateway'
import { isServerChosen, loadToken, setServerUrl } from './lib/api'
import type { ChosenStatus, PresenceStatus } from './lib/api'
import { isChannelMuted, isCommunityMuted, notify } from './lib/notify'
import { isSoundSyncing, markSoundSync, playMessageSound, playSound } from './lib/sounds'
import { loadChosenStatus, saveChosenStatus } from './lib/presence'
import { isNewer } from './lib/unread'
import { plainText } from './lib/markdownParser'
import { alertSummary, parseAlert } from './lib/automod'
import { reactionKey } from './lib/customEmoji'
import { applyEmojisUpdate } from './lib/emojiData'
import { knowsCategory, upsertCategory } from './lib/categories'
import { useFeedback } from './lib/feedbackStore'
import { isBlocked, useFriends } from './lib/friendsStore'
import { incomingCount } from './lib/friends'
import { joinNames } from './lib/dmList'
import { createRefreshBatch } from './lib/refreshBatch'
import { setCallUser, useCalls } from './lib/callStore'

/** Tipp-Anzeige verschwindet nach dieser Zeit ohne neues TYPING_START. */
const TYPING_TIMEOUT_MS = 8000
/** Eigene typing-Ops höchstens so oft senden (Server drosselt auf 3 s). */
const TYPING_SEND_INTERVAL_MS = 3500
/** Lesestand gesammelt an den Server melden. */
const READ_FLUSH_MS = 1000

const PAGE_SIZE = 50

export interface ChannelMessages {
  /** Aufsteigend sortiert (älteste zuerst). */
  items: Message[]
  hasMore: boolean
  loading: boolean
  /** Nach einem Sprung (Suche): Es gibt neuere Nachrichten, die noch nicht geladen sind. */
  hasNewer?: boolean
}

/** Geöffnete Suche: in einer Community (`communityId`) oder in den eigenen DMs (`null`). */
export interface SearchState {
  communityId: string | null
  /** Auf diesen Kanal beschränkt (optional). */
  channelId: string | null
  query: string
}

interface State {
  instance: InstanceInfo | null
  instanceError: boolean
  user: User | null
  /** Wurde die bestehende Session schon geprüft? */
  authChecked: boolean
  gatewayStatus: GatewayStatus

  communities: Community[]
  details: Record<string, CommunityDetail>
  members: Record<string, Member[]>
  messages: Record<string, ChannelMessages>
  /** Voice-States je Community (nur sichtbare Kanäle). */
  voiceStates: Record<string, VoiceState[]>

  selectedCommunityId: string | null
  selectedChannelId: string | null
  /** Einladungscode aus `/invite/{code}`, bis er angenommen oder verworfen wurde. */
  pendingInvite: string | null

  // --- Direktnachrichten & Suche ---
  /** Eigene DM-Kanäle (channel_id → DM). */
  dms: Record<string, DmChannel>
  /** Ist die Direktnachrichten-Ansicht statt einer Community geöffnet? */
  dmView: boolean
  selectedDmId: string | null
  /** Nach einem Sprung aus der Suche hervorzuhebende Nachricht. */
  jumpTarget: { channelId: string; messageId: string } | null
  search: SearchState | null

  loadDms: () => Promise<void>
  openDmView: () => void
  /** Freundesliste im DM-Bereich öffnen (keine Unterhaltung ausgewählt). */
  openFriends: () => void
  selectDm: (channelId: string) => void
  /** DM mit diesen Nutzern öffnen (1:1 wird wiederverwendet) und anzeigen. */
  openDm: (recipientIds: string[], name?: string) => Promise<void>
  /** Neue Gruppe anlegen (auch mit einer oder keiner weiteren Person) und anzeigen. */
  createGroupDm: (recipientIds: string[], name?: string) => Promise<void>
  /**
   * Leute zu einer Unterhaltung hinzufügen und die Unterhaltung anzeigen, in der sie jetzt sind: dieselbe Gruppe
   * bzw. bei einer 1:1-DM die neu entstandene Gruppe.
   */
  addToDm: (channelId: string, recipientIds: string[]) => Promise<void>
  /** Unterhaltung für sich anpinnen bzw. lösen. */
  setDmPinned: (channelId: string, pinned: boolean) => Promise<void>
  /** DM lokal übernehmen (z. B. nach Umbenennen). Der eigene Pin bleibt, außer die DM ist neu. */
  upsertDm: (dm: DmChannel) => void
  leaveDm: (channelId: string) => Promise<void>
  /** Zum Kanal einer Nachricht wechseln und die Nachrichten um sie herum laden. */
  jumpToMessage: (msg: Message) => Promise<void>
  clearJumpTarget: () => void
  /** Nach einem Sprung neuere Nachrichten nachladen. */
  loadNewer: (channelId: string) => Promise<void>
  openSearch: (search: SearchState) => void
  closeSearch: () => void

  init: () => Promise<void>
  /** Anmelden; bei aktiver 2FA kommt statt der Session eine Challenge zurück. */
  login: (username: string, password: string) => Promise<MfaChallenge | null>
  /** Zweiter Anmeldeschritt (2FA). */
  completeMfa: (mfaToken: string, method: MfaMethod, code: string) => Promise<void>
  /** Konto für eine neue OIDC-Identität anlegen. */
  oidcRegister: (data: Parameters<typeof endpoints.oidcRegister>[0]) => Promise<void>
  register: (data: {
    username: string
    password: string
    display_name?: string
    invite?: string
    email?: string
  }) => Promise<void>
  logout: () => Promise<void>
  setUser: (user: User) => void

  loadCommunities: () => Promise<void>
  loadDetail: (communityId: string) => Promise<void>
  loadMembers: (communityId: string) => Promise<void>
  selectCommunity: (communityId: string | null) => void
  selectChannel: (channelId: string) => void

  loadMessages: (channelId: string, older?: boolean) => Promise<void>
  sendMessage: (channelId: string, content: string, attachmentIds?: string[], replyToId?: string) => Promise<void>
  editMessage: (id: string, content: string) => Promise<void>
  deleteMessage: (id: string) => Promise<void>
  /** Eigene Reaktion umschalten (optimistisch; das Gateway-Echo ändert dann nichts mehr). */
  toggleReaction: (message: Message, emoji: string) => Promise<void>

  createCommunity: (name: string) => Promise<void>
  /** Community aus einer Discord-Servervorlage anlegen und hineinwechseln. */
  createCommunityFromTemplate: (code: string, name: string) => Promise<void>
  acceptInvite: (code: string) => Promise<void>
  clearPendingInvite: () => void
  updateCommunity: (community: Community) => void
  /** Rollenliste einer Community ersetzen (z. B. nach dem Umsortieren). */
  setRoles: (communityId: string, roles: Role[]) => void
  /** Community lokal entfernen (nach Löschen/Verlassen, ohne auf das Gateway zu warten). */
  forgetCommunity: (communityId: string) => void

  // --- Paket D: Präsenz, Tippen, Ungelesen, Instanzwahl ---
  /** Sichtbarer Status anderer Nutzer (user_id → Status). */
  presences: Record<string, PresenceStatus>
  /** Selbst gewählter Status (lokal gespeichert). */
  chosenStatus: ChosenStatus
  /** Automatisch abwesend nach Inaktivität. */
  autoIdle: boolean
  /** channel_id → (user_id → Ablaufzeitpunkt der Tipp-Anzeige in ms). */
  typing: Record<string, Record<string, number>>
  /** Desktop-Bundle: Wurde schon eine Instanz gewählt? (Im Web immer `true`.) */
  serverChosen: boolean

  setChosenStatus: (status: ChosenStatus) => void
  setAutoIdle: (idle: boolean) => void
  sendTyping: (channelId: string) => void
  /** Lesestand setzen; `communityId = null` für DMs. */
  markRead: (communityId: string | null, channelId: string, messageId: string) => void
  /** Kanal bis zur neuesten Nachricht als gelesen markieren (`communityId = null` für DMs). */
  markChannelRead: (communityId: string | null, channelId: string) => void
  /** Alle sichtbaren Textkanäle einer Community als gelesen markieren. */
  markCommunityRead: (communityId: string) => void
  /** Text, der in den Composer eines Kanals eingefügt werden soll (z. B. „@name “ aus einem Kontextmenü). */
  composerInsert: { channelId: string; text: string; nonce: number } | null
  insertIntoComposer: (channelId: string, text: string) => void
  clearComposerInsert: () => void
  chooseServer: (url: string) => Promise<void>
  changeServer: () => void

  loadVoiceStates: (communityId: string) => Promise<void>

  // --- Konto & Profil ---
  /** Geladene Profile anderer Nutzer (user_id → Profil), aktualisiert per USER_UPDATE. */
  profiles: Record<string, UserProfile>
  /** Geöffnete Profilkarte. */
  profileCard: ProfileCardState | null
  loadProfile: (userId: string) => Promise<void>
  openProfile: (userId: string, communityId: string | null, anchor: DOMRect) => void
  closeProfile: () => void
  /** Mitglied lokal ersetzen (z. B. nach Nickname-Änderung, ohne auf das Gateway zu warten). */
  upsertMember: (communityId: string, member: Member) => void
}

export interface ProfileCardState {
  userId: string
  communityId: string | null
  /** Position des auslösenden Elements (Viewport-Koordinaten). */
  anchor: { top: number; left: number; right: number; bottom: number }
}

let gateway: Gateway | null = null
/** Hat die aktuelle Gateway-Instanz schon ein READY geliefert? Ein weiteres READY bedeutet Reconnect. */
let readySeen = false

/** READY/RESYNC/PRESENCE_BULK enthalten nur Nutzer, die nicht offline sind – fehlend = offline. */
function presenceMap(list: { user_id: string; status: PresenceStatus }[] | undefined): Record<string, PresenceStatus> {
  return Object.fromEntries((list ?? []).map((p) => [p.user_id, p.status]))
}

/**
 * Index user_id → Mitglied je Mitgliederliste. Listen werden nie verändert, sondern ersetzt – der Index
 * gilt also, solange es dieselbe Liste gibt, und verschwindet mit ihr (WeakMap).
 */
const memberIndexes = new WeakMap<Member[], Map<string, Member>>()

/** Mitglied einer Liste per ID nachschlagen – O(1) statt `find` über alle Mitglieder (in Selektoren wichtig). */
export function memberOf(list: Member[] | undefined, userId: string | null | undefined): Member | undefined {
  if (!list || !userId) return undefined
  let index = memberIndexes.get(list)
  if (!index) {
    index = new Map(list.map((m) => [m.user.id, m]))
    memberIndexes.set(list, index)
  }
  return index.get(userId)
}

/** Eigener Status, wie er an den Server geht (gewählt, ggf. automatisch abwesend). */
function effectiveStatus(s: { chosenStatus: ChosenStatus; autoIdle: boolean }): ChosenStatus {
  return s.chosenStatus === 'online' && s.autoIdle ? 'idle' : s.chosenStatus
}

const lastTypingSent = new Map<string, number>()
/** channel_id → noch nicht gemeldeter Lesestand. */
const pendingReads = new Map<string, string>()
let readFlushTimer: number | undefined

function inviteFromPath(): string | null {
  const m = location.pathname.match(/^\/invite\/([^/]+)\/?$/)
  return m ? decodeURIComponent(m[1]) : null
}

/** Erwähnt die Nachricht den Nutzer (direkt oder per `@everyone`)? Eigene Nachrichten nie. */
export function mentionsUser(msg: Message, userId: string | undefined): boolean {
  if (!userId || msg.author?.id === userId) return false
  return !!msg.mention_everyone || (msg.mentions ?? []).includes(userId)
}

/**
 * Reaktion lokal anwenden. `self` = eigene Reaktion; dann nur, wenn sich `me` wirklich ändert.
 * Custom-Emotes (`name:id`) werden über ihre ID verglichen – der Name kann sich ändern.
 */
function applyReaction(msg: Message, emoji: string, add: boolean, self: boolean): Message {
  const reactions = msg.reactions ?? []
  const key = reactionKey(emoji)
  const same = (r: { emoji: string }) => reactionKey(r.emoji) === key
  const existing = reactions.find(same)
  if (self && (existing?.me ?? false) === add) return msg
  if (add) {
    const next = existing
      ? reactions.map((r) => (same(r) ? { ...r, count: r.count + 1, me: r.me || self } : r))
      : [...reactions, { emoji, count: 1, me: self }]
    return { ...msg, reactions: next }
  }
  if (!existing) return msg
  const next = reactions
    .map((r) => (same(r) ? { ...r, count: r.count - 1, me: self ? false : r.me } : r))
    .filter((r) => r.count > 0)
  return { ...msg, reactions: next }
}

function upsertMessage(list: Message[], msg: Message): Message[] {
  const idx = list.findIndex((m) => m.id === msg.id)
  if (idx >= 0) {
    const next = list.slice()
    next[idx] = msg
    return next
  }
  // UUIDv7 sind zeitlich sortierbar – Einfügen an der richtigen Stelle.
  const next = [...list, msg]
  next.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
  return next
}

/** Anzeigename einer DM: Gruppenname oder die anderen Teilnehmer. */
export function dmTitle(dm: DmChannel, meId: string | undefined): string {
  if (dm.name) return dm.name
  const others = dm.recipients.filter((u) => u.id !== meId)
  if (others.length === 0) return dm.kind === 'dm' ? 'Gelöschter Nutzer' : 'Neue Gruppe'
  return others.map((u) => displayName(u)).join(', ')
}

/**
 * Text der Systemnachricht `recipient_add` (ohne den Autor davor): „hat dich zur Gruppe hinzugefügt.“ bzw.
 * „hat Anna und Bert zur Gruppe hinzugefügt.“ Wer die Gruppe inzwischen verlassen hat, erscheint als „jemanden“.
 */
export function recipientAddText(msg: Message, dm: DmChannel | undefined, meId: string | undefined): string {
  const names = (msg.mentions ?? []).map((id) => {
    if (id === meId) return 'dich'
    const u = dm?.recipients.find((r) => r.id === id)
    return u ? displayName(u) : 'jemanden'
  })
  return `hat ${joinNames(names.length > 0 ? names : ['jemanden'])} zur Gruppe hinzugefügt.`
}

/** Anzeige eines Kanals in Texten: `#name` bei Textkanälen, sonst der Name der DM. */
export function channelLabel(channel: { kind: string; name: string }): string {
  return channel.kind === 'text' ? `#${channel.name}` : channel.name
}

/** Aktuellster Lesestand eines Kanals (Community oder DM). */
export function readStateOf(
  s: { details: Record<string, CommunityDetail>; dms: Record<string, DmChannel> },
  communityId: string | null,
  channelId: string,
): string | undefined {
  if (communityId === null) return s.dms[channelId]?.last_read_message_id ?? undefined
  return s.details[communityId]?.read_states?.[channelId]
}

/** Neueste bekannte Nachricht eines Kanals (Community oder DM). */
export function lastMessageOf(
  s: { details: Record<string, CommunityDetail>; dms: Record<string, DmChannel> },
  communityId: string | null,
  channelId: string,
): string | undefined {
  if (communityId === null) return s.dms[channelId]?.last_message_id ?? undefined
  return s.details[communityId]?.last_message_ids?.[channelId]
}

/** Ersten sichtbaren Textkanal einer Community wählen. */
function firstTextChannel(detail: CommunityDetail | undefined): string | null {
  if (!detail) return null
  const text = detail.channels.filter((c) => c.kind === 'text').sort((a, b) => a.position - b.position)
  return text[0]?.id ?? null
}

export const useStore = create<State>()((set, get) => {
  /** Rechte-Änderungen: Details gesammelt und zeitlich gestreut neu laden (siehe `refreshBatch`). */
  const detailRefresh = createRefreshBatch((ids) => {
    for (const id of ids) {
      if (get().details[id]) void get().loadDetail(id).catch(() => {})
    }
  })

  /**
   * Zustand nach verpassten Events (RESYNC) oder einem Reconnect komplett neu laden: Communities samt
   * Kanälen, Rechten, Lesestand und Voice-States (`loadDetail`), geladene Mitgliederlisten und den
   * offenen Kanal. Präsenz kommt mit READY bzw. RESYNC vom Server.
   */
  function resync() {
    // Lädt ohnehin alle Details neu.
    detailRefresh.cancel()
    set({ messages: {}, typing: {} })
    void get()
      .loadDms()
      .catch(() => {})
    void useFriends
      .getState()
      .load()
      .catch(() => {})
    void get()
      .loadCommunities()
      .then(() => {
        const s = get()
        // Ausgewählter Sprachkanal (Call-Bühne) hat keine Nachrichten.
        const voiceSelected =
          !s.dmView &&
          !!s.selectedCommunityId &&
          !!s.details[s.selectedCommunityId]?.channels.some((c) => c.id === s.selectedChannelId && c.kind === 'voice')
        const ch = s.dmView ? s.selectedDmId : voiceSelected ? null : s.selectedChannelId
        if (ch) void get().loadMessages(ch).catch(() => {})
        for (const cid of Object.keys(get().members)) {
          if (get().details[cid]) void get().loadMembers(cid).catch(() => {})
        }
      })
      .catch(() => {})
  }

  /** Rolle aus einem Gateway-Event übernehmen (neu oder geändert). */
  function upsertRole(role: Role) {
    set((s) => {
      const detail = s.details[role.community_id]
      if (!detail) return {}
      const exists = detail.roles.some((r) => r.id === role.id)
      const roles = exists ? detail.roles.map((r) => (r.id === role.id ? role : r)) : [...detail.roles, role]
      return { details: { ...s.details, [role.community_id]: { ...detail, roles } } }
    })
  }

  function startGateway() {
    gateway?.stop()
    readySeen = false
    gateway = new Gateway({
      onEvent: handleEvent,
      onStatus: (gatewayStatus) => set({ gatewayStatus }),
      onUnauthorized: () => void resetSession(),
      initialStatus: () => effectiveStatus(get()),
    })
    gateway.start()
  }

  /** Lesestand lokal übernehmen (nur vorwärts). */
  function applyReadState(communityId: string | null, channelId: string, messageId: string) {
    set((s) => {
      if (communityId === null) {
        const dm = s.dms[channelId]
        if (!dm || !isNewer(messageId, dm.last_read_message_id ?? undefined)) return {}
        // Bis zur neuesten Nachricht gelesen → keine ungelesenen Erwähnungen mehr.
        const readAll = !isNewer(dm.last_message_id ?? '', messageId)
        const mention_count = readAll ? 0 : dm.mention_count
        return { dms: { ...s.dms, [channelId]: { ...dm, last_read_message_id: messageId, mention_count } } }
      }
      const detail = s.details[communityId]
      if (!detail || !isNewer(messageId, detail.read_states?.[channelId])) return {}
      // Bis zur neuesten Nachricht gelesen → keine ungelesenen Erwähnungen mehr.
      let mention_counts = detail.mention_counts
      if (mention_counts?.[channelId] && !isNewer(detail.last_message_ids?.[channelId] ?? '', messageId)) {
        mention_counts = { ...mention_counts }
        delete mention_counts[channelId]
      }
      return {
        details: {
          ...s.details,
          [communityId]: {
            ...detail,
            read_states: { ...detail.read_states, [channelId]: messageId },
            mention_counts,
          },
        },
      }
    })
  }

  function flushReads() {
    readFlushTimer = undefined
    for (const [channelId, messageId] of pendingReads) {
      // Fehler (z. B. Nachricht inzwischen gelöscht) sind unkritisch – der nächste Lesestand kommt bestimmt.
      endpoints.markRead(channelId, messageId).catch(() => {})
    }
    pendingReads.clear()
  }

  function removeTyping(channelId: string, userId: string, onlyIfExpired: boolean) {
    set((s) => {
      const ch = s.typing[channelId]
      if (!ch || !(userId in ch) || (onlyIfExpired && ch[userId] > Date.now())) return {}
      const next = { ...ch }
      delete next[userId]
      return { typing: { ...s.typing, [channelId]: next } }
    })
  }

  /** Neue Nachricht: neueste ID merken, Tipp-Anzeige des Autors weg, ggf. Desktop-Benachrichtigung. */
  function onMessageCreated(msg: Message) {
    const s = get()
    if (msg.community_id === null) {
      onDmMessage(msg)
      return
    }
    const detail = s.details[msg.community_id]
    const mentioned = mentionsUser(msg, s.user?.id)
    if (detail && isNewer(msg.id, detail.last_message_ids?.[msg.channel_id])) {
      const counts = detail.mention_counts ?? {}
      set({
        details: {
          ...s.details,
          [msg.community_id]: {
            ...detail,
            last_message_ids: { ...detail.last_message_ids, [msg.channel_id]: msg.id },
            mention_counts: mentioned ? { ...counts, [msg.channel_id]: (counts[msg.channel_id] ?? 0) + 1 } : counts,
          },
        },
      })
    }
    const own = msg.author?.id === s.user?.id
    if (own) applyReadState(msg.community_id, msg.channel_id, msg.id)
    if (msg.author) removeTyping(msg.channel_id, msg.author.id, false)

    // Erwähnungen benachrichtigen auch aus stummgeschalteten Kanälen (nicht aber bei „Bitte nicht stören“).
    const wanted =
      (mentioned || (!isChannelMuted(msg.channel_id) && !isCommunityMuted(msg.community_id))) &&
      !isBlocked(msg.author?.id)
    if (!own && wanted && !document.hasFocus() && effectiveStatus(s) !== 'dnd') {
      const channel = detail?.channels.find((c) => c.id === msg.channel_id)
      const where = channel ? ` in #${channel.name}` : ''
      const nickname = memberOf(s.members[msg.community_id], msg.author?.id)?.nickname
      const who = nickname || senderName(msg)
      const alert = msg.kind === 'automod_alert' ? parseAlert(msg.content) : null
      const title = alert ? `AutoMod${where}` : mentioned ? `${who} hat dich erwähnt${where}` : `${who}${where}`
      const body =
        msg.kind === 'member_join'
          ? 'ist beigetreten.'
          : alert
            ? `${alertSummary(alert)}: ${who} · ${alert.rule_name}`
            : plainText(msg.content).slice(0, 200)
      void notify(title, body)
    }
  }

  /** Neue Nachricht in einer DM: neueste ID merken, unbekannte DM nachladen, ggf. benachrichtigen. */
  function onDmMessage(msg: Message) {
    const s = get()
    const dm = s.dms[msg.channel_id]
    const mentioned = mentionsUser(msg, s.user?.id)
    if (!dm) {
      void s.loadDms().catch(() => {})
    } else if (isNewer(msg.id, dm.last_message_id ?? undefined)) {
      const mention_count = dm.mention_count + (mentioned ? 1 : 0)
      set({ dms: { ...s.dms, [msg.channel_id]: { ...dm, last_message_id: msg.id, mention_count } } })
    }
    const own = msg.author?.id === s.user?.id
    if (own) applyReadState(null, msg.channel_id, msg.id)
    if (msg.author) removeTyping(msg.channel_id, msg.author.id, false)

    const wanted = (mentioned || !isChannelMuted(msg.channel_id)) && !isBlocked(msg.author?.id)
    if (!own && wanted && !document.hasFocus() && effectiveStatus(s) !== 'dnd') {
      const where = dm?.kind === 'group_dm' ? ` in ${dmTitle(dm, s.user?.id)}` : ''
      const body =
        msg.kind === 'call_missed'
          ? 'Verpasster Anruf'
          : msg.kind === 'recipient_add'
            ? recipientAddText(msg, dm, s.user?.id)
            : plainText(msg.content).slice(0, 200)
      void notify(`${senderName(msg)}${where}`, body)
    }
  }

  function resetSession() {
    detailRefresh.cancel()
    gateway?.stop()
    gateway = null
    setToken(null)
    useFriends.getState().reset()
    useCalls.getState().reset()
    setCallUser(undefined)
    set({
      user: null,
      communities: [],
      details: {},
      members: {},
      messages: {},
      voiceStates: {},
      selectedCommunityId: null,
      selectedChannelId: null,
      gatewayStatus: 'offline',
      presences: {},
      typing: {},
      dms: {},
      dmView: false,
      selectedDmId: null,
      jumpTarget: null,
      search: null,
      profiles: {},
      profileCard: null,
      composerInsert: null,
    })
  }

  /** Profiländerung überall übernehmen: eigenes Konto, Mitgliederlisten, Autoren geladener Nachrichten. */
  function applyUserUpdate(p: UserProfile) {
    set((s) => {
      const pub: UserPublic = {
        id: p.id,
        username: p.username,
        display_name: p.display_name,
        avatar_url: p.avatar_url,
        ...(p.bot ? { bot: true } : {}),
      }
      const members: Record<string, Member[]> = {}
      for (const [cid, list] of Object.entries(s.members)) {
        members[cid] = list.some((m) => m.user.id === p.id)
          ? list.map((m) => (m.user.id === p.id ? { ...m, user: pub } : m))
          : list
      }
      const messages: Record<string, ChannelMessages> = {}
      for (const [cid, ch] of Object.entries(s.messages)) {
        // Auch zitierte Autoren in Antworten aktualisieren, sonst zeigt die Antwortzeile den alten Namen.
        const touches = (m: Message) => m.author?.id === p.id || m.referenced_message?.author?.id === p.id
        messages[cid] = ch.items.some(touches)
          ? {
              ...ch,
              items: ch.items.map((m) => {
                if (!touches(m)) return m
                const ref = m.referenced_message
                return {
                  ...m,
                  author: m.author?.id === p.id ? pub : m.author,
                  referenced_message: ref?.author?.id === p.id ? { ...ref, author: pub } : ref,
                }
              }),
            }
          : ch
      }
      // DM-Teilnehmerlisten (Name, Avatar).
      const dms: Record<string, DmChannel> = {}
      for (const [id, dm] of Object.entries(s.dms)) {
        dms[id] = dm.recipients.some((u) => u.id === p.id)
          ? { ...dm, recipients: dm.recipients.map((u) => (u.id === p.id ? pub : u)) }
          : dm
      }
      return {
        profiles: { ...s.profiles, [p.id]: p },
        members,
        messages,
        dms,
        user:
          s.user?.id === p.id
            ? {
                ...s.user,
                ...pub,
                bio: p.bio,
                status_text: p.status_text,
                pronouns: p.pronouns,
                banner_url: p.banner_url,
                banner_color: p.banner_color,
                accent_color: p.accent_color,
              }
            : s.user,
      }
    })
  }

  async function afterAuth(res: AuthResponse) {
    if (res.token) setToken(res.token)
    set({ user: res.user })
    startGateway()
    void get()
      .loadDms()
      .catch(() => {})
    void useFriends
      .getState()
      .load()
      .catch(() => {})
    await get().loadCommunities()
  }

  function removeCommunity(id: string) {
    set((s) => {
      const details = { ...s.details }
      delete details[id]
      const members = { ...s.members }
      delete members[id]
      const voiceStates = { ...s.voiceStates }
      delete voiceStates[id]
      const communities = s.communities.filter((c) => c.id !== id)
      const selected = s.selectedCommunityId === id
      return {
        communities,
        details,
        members,
        voiceStates,
        selectedCommunityId: selected ? (communities[0]?.id ?? null) : s.selectedCommunityId,
        selectedChannelId: selected ? null : s.selectedChannelId,
      }
    })
    const next = get().selectedCommunityId
    if (next && !get().selectedChannelId) get().selectCommunity(next)
  }

  function handleEvent(ev: GatewayEvent) {
    const s = get()
    switch (ev.t) {
      case 'READY': {
        markSoundSync()
        setCallUser(ev.d.user.id)
        useCalls.getState().setAll(ev.d.dm_calls)
        set({
          user: ev.d.user,
          presences: presenceMap(ev.d.presences),
        })
        if (readySeen) {
          // Wiederverbunden: Während der Unterbrechung verpasste Events lassen sich nicht nachholen.
          resync()
        } else {
          const known = new Set(s.communities.map((c) => c.id))
          if (ev.d.community_ids.length !== known.size || ev.d.community_ids.some((id) => !known.has(id))) {
            void s.loadCommunities()
          }
        }
        const dmIds = ev.d.dm_channel_ids ?? []
        if (dmIds.length !== Object.keys(s.dms).length || dmIds.some((id) => !s.dms[id])) {
          void s.loadDms().catch(() => {})
        }
        readySeen = true
        break
      }
      case 'MESSAGE_CREATE':
      case 'MESSAGE_UPDATE': {
        if (ev.t === 'MESSAGE_CREATE') onMessageCreated(ev.d)
        // Nachrichten blockierter Nutzer (Gruppen-DMs, Communities) machen keinen Ton.
        if (ev.t === 'MESSAGE_CREATE' && !isBlocked(ev.d.author?.id)) playMessageSound(ev.d, s)
        const ch = s.messages[ev.d.channel_id]
        if (!ch) break
        if (ev.t === 'MESSAGE_CREATE') {
          // Nach einem Sprung keine Lücke erzeugen: neuere Nachrichten werden ohnehin nachgeladen.
          if (ch.hasNewer) break
          set({ messages: { ...s.messages, [ev.d.channel_id]: { ...ch, items: upsertMessage(ch.items, ev.d) } } })
          break
        }
        // Updates nur für bekannte Nachrichten. `reactions[].me` ist in Events immer false – den eigenen
        // Reaktionsstand behalten (er wird über die Reaktions-Events gepflegt).
        const known = ch.items.find((m) => m.id === ev.d.id)
        if (!known) break
        const updated: Message = { ...ev.d, reactions: known.reactions }
        const items = upsertMessage(ch.items, updated).map((m) =>
          m.referenced_message?.id === updated.id
            ? {
                ...m,
                referenced_message: {
                  ...m.referenced_message,
                  content: updated.content.slice(0, 200),
                  has_attachments: updated.attachments.length > 0,
                },
              }
            : m,
        )
        set({ messages: { ...s.messages, [ev.d.channel_id]: { ...ch, items } } })
        break
      }
      case 'MESSAGE_DELETE': {
        const ch = s.messages[ev.d.channel_id]
        if (!ch) break
        const items = ch.items
          .filter((m) => m.id !== ev.d.id)
          // Antworten bleiben, ihr Verweis zeigt dann „Ursprüngliche Nachricht gelöscht“.
          .map((m) => (m.referenced_message?.id === ev.d.id ? { ...m, referenced_message: null } : m))
        set({ messages: { ...s.messages, [ev.d.channel_id]: { ...ch, items } } })
        break
      }
      case 'MESSAGE_REACTION_ADD':
      case 'MESSAGE_REACTION_REMOVE': {
        const ch = s.messages[ev.d.channel_id]
        if (!ch) break
        const { message_id, emoji, user_id } = ev.d
        const add = ev.t === 'MESSAGE_REACTION_ADD'
        const self = user_id === s.user?.id
        const items = ch.items.map((m) => (m.id === message_id ? applyReaction(m, emoji, add, self) : m))
        set({ messages: { ...s.messages, [ev.d.channel_id]: { ...ch, items } } })
        break
      }
      case 'CHANNEL_CREATE':
        // Rechte für den neuen Kanal kennt nur der Server – Detail neu laden.
        void s.loadDetail(ev.d.community_id)
        break
      case 'CHANNEL_UPDATE': {
        const detail = s.details[ev.d.community_id]
        if (!detail) break
        // In eine Kategorie verschoben, die dieser Client noch nicht kennt (war für ihn unsichtbar)?
        if (!knowsCategory(detail.categories, ev.d.category_id)) {
          void s.loadDetail(ev.d.community_id)
          break
        }
        set({
          details: {
            ...s.details,
            [ev.d.community_id]: { ...detail, channels: detail.channels.map((c) => (c.id === ev.d.id ? ev.d : c)) },
          },
        })
        break
      }
      case 'CHANNEL_DELETE': {
        const detail = s.details[ev.d.community_id]
        if (detail) {
          const next = { ...detail, channels: detail.channels.filter((c) => c.id !== ev.d.id) }
          set({ details: { ...s.details, [ev.d.community_id]: next } })
          if (s.selectedChannelId === ev.d.id) set({ selectedChannelId: firstTextChannel(next) })
        }
        const messages = { ...get().messages }
        delete messages[ev.d.id]
        set({ messages })
        const voice = get().voiceStates[ev.d.community_id]
        if (voice?.some((v) => v.channel_id === ev.d.id)) {
          set({
            voiceStates: { ...get().voiceStates, [ev.d.community_id]: voice.filter((v) => v.channel_id !== ev.d.id) },
          })
        }
        break
      }
      case 'CATEGORY_CREATE':
      case 'CATEGORY_UPDATE': {
        const detail = s.details[ev.d.community_id]
        if (!detail) break
        set({
          details: {
            ...s.details,
            [ev.d.community_id]: { ...detail, categories: upsertCategory(detail.categories, ev.d) },
          },
        })
        break
      }
      case 'CATEGORY_DELETE': {
        const detail = s.details[ev.d.community_id]
        if (!detail) break
        const next = {
          ...detail,
          categories: (detail.categories ?? []).filter((c) => c.id !== ev.d.id),
          channels: detail.channels.map((c) => (c.category_id === ev.d.id ? { ...c, category_id: null } : c)),
        }
        set({ details: { ...s.details, [ev.d.community_id]: next } })
        break
      }
      case 'COMMUNITY_UPDATE':
        s.updateCommunity(ev.d)
        break
      case 'COMMUNITY_DELETE':
        removeCommunity(ev.d.id)
        break
      case 'MEMBER_ADD':
      case 'MEMBER_UPDATE': {
        const { community_id, member } = ev.d
        if (member.user.id === s.user?.id) {
          if (ev.t === 'MEMBER_ADD') void s.loadCommunities()
          else void s.loadDetail(community_id)
        }
        // Neuer Anzeigename auch in DMs übernehmen.
        if (Object.values(s.dms).some((dm) => dm.recipients.some((u) => u.id === member.user.id))) {
          const dms: Record<string, DmChannel> = {}
          for (const [id, dm] of Object.entries(s.dms)) {
            dms[id] = { ...dm, recipients: dm.recipients.map((u) => (u.id === member.user.id ? member.user : u)) }
          }
          set({ dms })
        }
        const list = s.members[community_id]
        if (list) {
          const exists = list.some((m) => m.user.id === member.user.id)
          set({
            members: {
              ...s.members,
              [community_id]: exists ? list.map((m) => (m.user.id === member.user.id ? member : m)) : [...list, member],
            },
          })
        }
        break
      }
      case 'USER_UPDATE':
        applyUserUpdate(ev.d)
        break
      case 'MEMBER_REMOVE': {
        const { community_id, user_id } = ev.d
        if (user_id === s.user?.id) {
          removeCommunity(community_id)
          break
        }
        const list = s.members[community_id]
        if (list) set({ members: { ...s.members, [community_id]: list.filter((m) => m.user.id !== user_id) } })
        break
      }
      case 'ROLE_CREATE':
      case 'ROLE_UPDATE':
        // Rechte-Änderungen kommen zusätzlich als PERMISSIONS_CHANGED – hier genügt es, die Rolle zu übernehmen
        // (spart beim Umsortieren einen Detail-Abruf pro Rolle).
        upsertRole(ev.d)
        break
      case 'ROLE_DELETE':
      case 'PERMISSIONS_CHANGED':
        // Geht an alle Mitglieder gleichzeitig – gesammelt und gestreut neu laden statt sofort.
        detailRefresh.add(ev.d.community_id)
        break
      case 'VOICE_STATE_UPDATE': {
        // Ein Nutzer ist nur in einem Voice-Kanal bzw. DM-Anruf: aus allen anderen Listen entfernen.
        // DM-Anrufe (`community_id: null`) führt `lib/callStore.ts`.
        useCalls.getState().applyVoiceState(ev.d)
        const cid = ev.d.community_id
        const voiceStates: Record<string, VoiceState[]> = {}
        for (const [id, list] of Object.entries(s.voiceStates)) {
          voiceStates[id] = list.filter((v) => v.user_id !== ev.d.user_id)
        }
        if (cid !== null) {
          const list = voiceStates[cid] ?? []
          const prev = s.voiceStates[cid]?.find((v) => v.user_id === ev.d.user_id)
          voiceStates[cid] = prev
            ? (s.voiceStates[cid] ?? []).map((v) => (v.user_id === ev.d.user_id ? ev.d : v))
            : [...list, ev.d]
        }
        set({ voiceStates })
        break
      }
      case 'CALL_UPDATE':
        useCalls.getState().applyUpdate(ev.d)
        break
      case 'VOICE_STATE_DELETE': {
        if (ev.d.community_id === null) {
          useCalls.getState().removeVoiceState(ev.d)
          break
        }
        const list = s.voiceStates[ev.d.community_id]
        if (!list) break
        set({
          voiceStates: {
            ...s.voiceStates,
            [ev.d.community_id]: list.filter((v) => !(v.user_id === ev.d.user_id && v.channel_id === ev.d.channel_id)),
          },
        })
        break
      }
      case 'RESYNC':
        markSoundSync()
        // Präsenz gibt es nur über das Gateway – der Server schickt den aktuellen Stand mit.
        if (ev.d.presences) set({ presences: presenceMap(ev.d.presences) })
        if (ev.d.dm_calls) useCalls.getState().setAll(ev.d.dm_calls)
        resync()
        break
      case 'PONG':
        break
      case 'PRESENCE_UPDATE':
        set({ presences: { ...s.presences, [ev.d.user_id]: ev.d.status } })
        break
      case 'PRESENCE_BULK':
        // Nach eigenem Beitritt: Status der neuen Mitglieder einmischen.
        set({ presences: { ...s.presences, ...presenceMap(ev.d.presences) } })
        break
      case 'TYPING_START': {
        const { channel_id, user_id } = ev.d
        if (user_id === s.user?.id) break
        set({ typing: { ...s.typing, [channel_id]: { ...s.typing[channel_id], [user_id]: Date.now() + TYPING_TIMEOUT_MS } } })
        window.setTimeout(() => removeTyping(channel_id, user_id, true), TYPING_TIMEOUT_MS + 50)
        break
      }
      case 'READ_STATE_UPDATE':
        applyReadState(ev.d.community_id, ev.d.channel_id, ev.d.last_read_message_id)
        break
      case 'DM_CREATE':
      case 'DM_UPDATE':
        s.upsertDm(ev.d)
        break
      case 'DM_DELETE':
        removeDm(ev.d.id)
        break
      case 'DM_PIN_UPDATE':
        applyDmPin(ev.d.channel_id, ev.d.pinned_at)
        break
      case 'EMOJIS_UPDATE':
        applyEmojisUpdate(ev.d)
        break
      case 'RELATIONSHIP_ADD': {
        const prev = useFriends.getState().relationships[ev.d.id]?.type
        useFriends.getState().applyAdd(ev.d)
        if (ev.d.type === 'pending_in' && prev !== 'pending_in') onFriendRequest(ev.d.user)
        break
      }
      case 'RELATIONSHIP_REMOVE':
        useFriends.getState().applyRemove(ev.d.id)
        break
      case 'FEEDBACK_UPDATE':
        useFeedback.getState().applyUpdate(ev.d)
        break
      case 'FEEDBACK_DELETE':
        useFeedback.getState().applyDelete(ev.d.id)
        break
      // Instanz-Verwaltung (Paket H): Einstellungen/Ankündigung geändert bzw. eigenes Admin-Recht.
      case 'INSTANCE_UPDATE':
        set({ instance: ev.d })
        break
      case 'INSTANCE_ADMIN_UPDATE':
        if (s.user) set({ user: { ...s.user, is_instance_admin: ev.d.is_instance_admin } })
        break
    }
  }

  /** Neue eingehende Freundschaftsanfrage: Ton und – im Hintergrund – Desktop-Benachrichtigung. */
  function onFriendRequest(from: UserPublic) {
    const s = get()
    if (effectiveStatus(s) === 'dnd' || isSoundSyncing()) return
    playSound('friendRequest')
    if (!document.hasFocus()) void notify('Neue Freundschaftsanfrage', `${displayName(from)} möchte mit dir befreundet sein.`)
  }

  /** Eigenen Pin einer DM übernehmen (Antwort auf Anpinnen/Lösen oder `DM_PIN_UPDATE` von einem anderen Gerät). */
  function applyDmPin(id: string, pinnedAt: string | null) {
    set((s) => {
      const dm = s.dms[id]
      if (!dm || (dm.pinned_at ?? null) === pinnedAt) return {}
      return { dms: { ...s.dms, [id]: { ...dm, pinned_at: pinnedAt } } }
    })
  }

  function removeDm(id: string) {
    useCalls.getState().forget(id)
    set((s) => {
      const dms = { ...s.dms }
      delete dms[id]
      const messages = { ...s.messages }
      delete messages[id]
      return { dms, messages, selectedDmId: s.selectedDmId === id ? null : s.selectedDmId }
    })
  }

  return {
    instance: null,
    instanceError: false,
    user: null,
    authChecked: false,
    gatewayStatus: 'offline',
    communities: [],
    details: {},
    members: {},
    messages: {},
    voiceStates: {},
    selectedCommunityId: null,
    selectedChannelId: null,
    pendingInvite: inviteFromPath(),
    dms: {},
    dmView: false,
    selectedDmId: null,
    jumpTarget: null,
    search: null,
    presences: {},
    chosenStatus: loadChosenStatus(),
    autoIdle: false,
    typing: {},
    serverChosen: isServerChosen(),
    profiles: {},
    profileCard: null,
    composerInsert: null,

    async init() {
      // Desktop-Bundle: erst Instanz wählen lassen (ServerSetup), dann Token aus dem Schlüsselbund holen.
      if (!get().serverChosen) return
      await loadToken()
      endpoints
        .instance()
        .then((instance) => set({ instance, instanceError: false }))
        .catch(() => set({ instanceError: true }))
      try {
        const user = await endpoints.me()
        set({ user, authChecked: true })
        startGateway()
        void get()
          .loadDms()
          .catch(() => {})
        void useFriends
          .getState()
          .load()
          .catch(() => {})
        await get().loadCommunities()
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) setToken(null)
        set({ authChecked: true })
      }
    },

    async login(username, password) {
      const res = await endpoints.login({ username, password })
      if (isMfaChallenge(res)) return res
      await afterAuth(res)
      return null
    },

    async completeMfa(mfaToken, method, code) {
      await afterAuth(await endpoints.loginMfa({ mfa_token: mfaToken, method, code }))
    },

    async oidcRegister(data) {
      const res = await endpoints.oidcRegister(data)
      if (data.invite && get().pendingInvite === data.invite) get().clearPendingInvite()
      await afterAuth(res)
    },

    async register(data) {
      const res = await endpoints.register(data)
      // Registrierung mit Einladung tritt der Community direkt bei; /invite/… aus der Adresse nehmen, sonst
      // erscheint nach einem Neuladen der Einladungsdialog erneut.
      if (data.invite && get().pendingInvite === data.invite) get().clearPendingInvite()
      await afterAuth(res)
    },

    async logout() {
      try {
        await endpoints.logout()
      } catch {
        // Session ist ggf. schon weg – lokal trotzdem abmelden.
      }
      resetSession()
      // Instanzinfo neu laden (bootstrap kann sich geändert haben).
      endpoints.instance().then((instance) => set({ instance })).catch(() => {})
    },

    setUser(user) {
      set({ user })
      // Eigene Änderung sofort überall zeigen (das Gateway-Event folgt).
      applyUserUpdate({
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url ?? null,
        bio: user.bio ?? null,
        status_text: user.status_text ?? null,
        created_at: user.created_at,
        // Ohne diese Felder setzte applyUserUpdate sie am eigenen Konto auf undefined (Gestaltung galt dann als
        // ungespeichert, Banner/Farben verschwanden bis zum nächsten Gateway-Event).
        pronouns: user.pronouns ?? null,
        banner_url: user.banner_url ?? null,
        banner_color: user.banner_color ?? null,
        accent_color: user.accent_color ?? null,
      })
    },

    async loadCommunities() {
      const communities = await endpoints.communities()
      set({ communities })
      await Promise.all(communities.map((c) => get().loadDetail(c.id)))
      const { selectedCommunityId } = get()
      if (!selectedCommunityId || !communities.some((c) => c.id === selectedCommunityId)) {
        get().selectCommunity(communities[0]?.id ?? null)
      }
    },

    async loadDetail(communityId) {
      try {
        const detail = await endpoints.community(communityId)
        set((s) => ({
          details: { ...s.details, [communityId]: detail },
          communities: s.communities.some((c) => c.id === communityId)
            ? s.communities.map((c) => (c.id === communityId ? detail.community : c))
            : [...s.communities, detail.community],
        }))
        const s = get()
        if (s.selectedCommunityId === communityId) {
          const stillVisible = detail.channels.some((c) => c.id === s.selectedChannelId)
          if (!stillVisible) set({ selectedChannelId: firstTextChannel(detail) })
        }
        // Sichtbarkeit kann sich geändert haben – Voice-States mitladen.
        void get()
          .loadVoiceStates(communityId)
          .catch(() => {})
      } catch (e) {
        if (e instanceof ApiError && (e.status === 403 || e.status === 404)) removeCommunity(communityId)
        else throw e
      }
    },

    async loadMembers(communityId) {
      const list = await endpoints.members(communityId)
      set((s) => ({ members: { ...s.members, [communityId]: list } }))
    },

    selectCommunity(communityId) {
      set({
        dmView: false,
        profileCard: null,
        selectedCommunityId: communityId,
        selectedChannelId: communityId ? firstTextChannel(get().details[communityId]) : null,
      })
    },

    selectChannel(channelId) {
      set({ selectedChannelId: channelId, profileCard: null })
    },

    async loadMessages(channelId, older = false) {
      const current = get().messages[channelId]
      if (current?.loading) return
      if (older && (!current || !current.hasMore)) return
      const before = older ? current?.items[0]?.id : undefined
      set((s) => ({
        messages: {
          ...s.messages,
          [channelId]: { items: older ? (current?.items ?? []) : [], hasMore: true, loading: true },
        },
      }))
      try {
        const page = await endpoints.messages(channelId, before, PAGE_SIZE)
        const asc = page.slice().reverse()
        set((s) => {
          const prev = s.messages[channelId]?.items ?? []
          // Während des Ladens per Gateway eingetroffene Nachrichten nicht verlieren.
          let items = older ? [...asc, ...prev] : asc
          if (!older) for (const m of prev) items = upsertMessage(items, m)
          return { messages: { ...s.messages, [channelId]: { items, hasMore: page.length === PAGE_SIZE, loading: false } } }
        })
      } catch (e) {
        set((s) => ({
          messages: {
            ...s.messages,
            [channelId]: { items: s.messages[channelId]?.items ?? [], hasMore: false, loading: false },
          },
        }))
        throw e
      }
    },

    async sendMessage(channelId, content, attachmentIds, replyToId) {
      const msg = await endpoints.sendMessage(channelId, content, attachmentIds, replyToId)
      if (get().messages[channelId]?.hasNewer) {
        // Nach einem Sprung: zum aktuellen Ende wechseln statt eine Lücke zu erzeugen.
        set((s) => {
          const messages = { ...s.messages }
          delete messages[channelId]
          return { messages }
        })
        void get().loadMessages(channelId)
      }
      set((s) => {
        const ch = s.messages[channelId] ?? { items: [], hasMore: false, loading: false }
        return { messages: { ...s.messages, [channelId]: { ...ch, items: upsertMessage(ch.items, msg) } } }
      })
      // Eigene Nachricht gilt als gelesen; Tipp-Drosselung zurücksetzen.
      lastTypingSent.delete(channelId)
      get().markRead(msg.community_id, channelId, msg.id)
    },

    async editMessage(id, content) {
      const msg = await endpoints.editMessage(id, content)
      set((s) => {
        const ch = s.messages[msg.channel_id]
        if (!ch) return {}
        return { messages: { ...s.messages, [msg.channel_id]: { ...ch, items: upsertMessage(ch.items, msg) } } }
      })
    },

    async deleteMessage(id) {
      await endpoints.deleteMessage(id)
      set((s) => {
        const messages = { ...s.messages }
        for (const [cid, ch] of Object.entries(messages)) {
          if (ch.items.some((m) => m.id === id)) messages[cid] = { ...ch, items: ch.items.filter((m) => m.id !== id) }
        }
        return { messages }
      })
    },

    async toggleReaction(message, emoji) {
      const current = get().messages[message.channel_id]?.items.find((m) => m.id === message.id) ?? message
      const add = !current.reactions?.some((r) => reactionKey(r.emoji) === reactionKey(emoji) && r.me)
      const apply = (adding: boolean) =>
        set((s) => {
          const ch = s.messages[message.channel_id]
          if (!ch) return {}
          const items = ch.items.map((m) => (m.id === message.id ? applyReaction(m, emoji, adding, true) : m))
          return { messages: { ...s.messages, [message.channel_id]: { ...ch, items } } }
        })
      apply(add)
      try {
        if (add) await endpoints.addReaction(message.id, emoji)
        else await endpoints.removeReaction(message.id, emoji)
      } catch (e) {
        apply(!add)
        throw e
      }
    },

    async createCommunityFromTemplate(code, name) {
      const detail = await endpoints.createCommunityFromTemplate(code, name)
      set((s) => ({
        communities: s.communities.some((c) => c.id === detail.community.id)
          ? s.communities
          : [...s.communities, detail.community],
        details: { ...s.details, [detail.community.id]: detail },
      }))
      get().selectCommunity(detail.community.id)
    },

    async createCommunity(name) {
      const detail = await endpoints.createCommunity(name)
      set((s) => ({
        communities: s.communities.some((c) => c.id === detail.community.id)
          ? s.communities
          : [...s.communities, detail.community],
        details: { ...s.details, [detail.community.id]: detail },
      }))
      get().selectCommunity(detail.community.id)
    },

    async acceptInvite(code) {
      let detail: CommunityDetail
      try {
        detail = await endpoints.acceptInvite(code)
      } catch (e) {
        if (!(e instanceof ApiError && e.code === 'already_member')) throw e
        const community = (await endpoints.invite(code)).community
        if (!community) throw e
        await get().loadDetail(community.id)
        get().clearPendingInvite()
        get().selectCommunity(community.id)
        return
      }
      set((s) => ({
        communities: s.communities.some((c) => c.id === detail.community.id)
          ? s.communities
          : [...s.communities, detail.community],
        details: { ...s.details, [detail.community.id]: detail },
      }))
      get().clearPendingInvite()
      get().selectCommunity(detail.community.id)
    },

    clearPendingInvite() {
      set({ pendingInvite: null })
      if (location.pathname.startsWith('/invite/')) history.replaceState(null, '', '/')
    },

    async loadVoiceStates(communityId) {
      const list = await endpoints.voiceStates(communityId)
      set((s) => ({ voiceStates: { ...s.voiceStates, [communityId]: list } }))
    },

    updateCommunity(community) {
      set((s) => ({
        communities: s.communities.map((c) => (c.id === community.id ? community : c)),
        details: s.details[community.id]
          ? { ...s.details, [community.id]: { ...s.details[community.id], community } }
          : s.details,
      }))
    },

    setRoles(communityId, roles) {
      set((s) => {
        const detail = s.details[communityId]
        return detail ? { details: { ...s.details, [communityId]: { ...detail, roles } } } : {}
      })
    },

    forgetCommunity(communityId) {
      removeCommunity(communityId)
    },

    setChosenStatus(chosenStatus) {
      saveChosenStatus(chosenStatus)
      set({ chosenStatus })
      gateway?.send({ op: 'presence', status: effectiveStatus(get()) })
    },

    setAutoIdle(autoIdle) {
      if (get().autoIdle === autoIdle) return
      set({ autoIdle })
      if (get().chosenStatus === 'online') gateway?.send({ op: 'presence', status: effectiveStatus(get()) })
    },

    sendTyping(channelId) {
      const now = Date.now()
      if (now - (lastTypingSent.get(channelId) ?? 0) < TYPING_SEND_INTERVAL_MS) return
      lastTypingSent.set(channelId, now)
      gateway?.send({ op: 'typing', channel_id: channelId })
    },

    markRead(communityId, channelId, messageId) {
      const s = get()
      const known = communityId === null ? !!s.dms[channelId] : !!s.details[communityId]
      if (!known || !isNewer(messageId, readStateOf(s, communityId, channelId))) return
      applyReadState(communityId, channelId, messageId)
      pendingReads.set(channelId, messageId)
      readFlushTimer ??= window.setTimeout(flushReads, READ_FLUSH_MS)
    },

    markChannelRead(communityId, channelId) {
      const last = lastMessageOf(get(), communityId, channelId)
      if (last) get().markRead(communityId, channelId, last)
    },

    markCommunityRead(communityId) {
      const detail = get().details[communityId]
      for (const c of detail?.channels ?? []) {
        if (c.kind === 'text') get().markChannelRead(communityId, c.id)
      }
    },

    insertIntoComposer(channelId, text) {
      set({ composerInsert: { channelId, text, nonce: Date.now() } })
    },

    clearComposerInsert() {
      set({ composerInsert: null })
    },

    async chooseServer(url) {
      // Nur im abgemeldeten Zustand erreichbar; das Token der neuen Instanz kommt aus dem Schlüsselbund.
      await setServerUrl(url)
      set({ serverChosen: true, authChecked: false, instance: null, instanceError: false })
      await get().init()
    },

    changeServer() {
      set({ serverChosen: false })
    },

    async loadDms() {
      const list = await endpoints.dms()
      set((s) => {
        const dms = Object.fromEntries(list.map((dm) => [dm.id, dm]))
        return { dms, selectedDmId: s.selectedDmId && dms[s.selectedDmId] ? s.selectedDmId : null }
      })
    },

    openDmView() {
      // Ohne Auswahl die zuletzt aktive Unterhaltung öffnen – bei offenen Freundschaftsanfragen die Freundesliste.
      const { selectedDmId, dms } = get()
      const recent = Object.values(dms)
        .filter((dm) => dm.last_message_id || dm.kind === 'group_dm')
        .sort((a, b) => ((a.last_message_id ?? a.id) < (b.last_message_id ?? b.id) ? 1 : -1))[0]
      const requests = incomingCount(Object.values(useFriends.getState().relationships)) > 0
      const fallback = requests ? null : (recent?.id ?? null)
      set({ dmView: true, profileCard: null, selectedDmId: selectedDmId ?? fallback })
    },

    openFriends() {
      set({ dmView: true, profileCard: null, selectedDmId: null })
    },

    selectDm(channelId) {
      set({ dmView: true, profileCard: null, selectedDmId: channelId })
    },

    async openDm(recipientIds, name) {
      const dm = await endpoints.createDm(recipientIds, name)
      get().upsertDm(dm)
      get().selectDm(dm.id)
    },

    async createGroupDm(recipientIds, name) {
      const dm = await endpoints.createGroupDm(recipientIds, name)
      get().upsertDm(dm)
      get().selectDm(dm.id)
    },

    async addToDm(channelId, recipientIds) {
      const dm = await endpoints.addDmRecipients(channelId, recipientIds)
      get().upsertDm(dm)
      get().selectDm(dm.id)
    },

    async setDmPinned(channelId, pinned) {
      // Die Antwort (bzw. `DM_PIN_UPDATE`) setzt den Stand; ohne optimistisches Umsortieren springt nichts zurück.
      let pinnedAt: string | null = null
      if (pinned) pinnedAt = (await endpoints.pinDm(channelId)).pinned_at ?? null
      else await endpoints.unpinDm(channelId)
      applyDmPin(channelId, pinnedAt)
    },

    upsertDm(dm) {
      set((s) => {
        const prev = s.dms[dm.id]
        // Events tragen weder Lesestand noch Erwähnungen – die eigenen behalten.
        const lastRead = dm.last_read_message_id ?? prev?.last_read_message_id ?? null
        const mentionCount = dm.last_read_message_id ? dm.mention_count : (prev?.mention_count ?? dm.mention_count)
        const last =
          prev?.last_message_id && isNewer(prev.last_message_id, dm.last_message_id ?? undefined)
            ? prev.last_message_id
            : dm.last_message_id
        // Events tragen auch den eigenen Pin nicht; er ändert sich nur über `DM_PIN_UPDATE` bzw. `setDmPinned`.
        const pinnedAt = prev ? (prev.pinned_at ?? null) : (dm.pinned_at ?? null)
        return {
          dms: {
            ...s.dms,
            [dm.id]: {
              ...dm,
              last_read_message_id: lastRead,
              last_message_id: last,
              mention_count: mentionCount,
              pinned_at: pinnedAt,
            },
          },
        }
      })
    },

    async leaveDm(channelId) {
      await endpoints.leaveDm(channelId)
      removeDm(channelId)
    },

    async jumpToMessage(msg) {
      const channelId = msg.channel_id
      const loaded = get().messages[channelId]?.items.some((m) => m.id === msg.id)
      if (!loaded) {
        // Platzhalter mit `loading`, damit die Nachrichtenansicht nicht parallel das Kanalende lädt.
        set((s) => ({ messages: { ...s.messages, [channelId]: { items: [], hasMore: true, loading: true } } }))
      }
      if (msg.community_id === null) set({ dmView: true, selectedDmId: channelId })
      else set({ dmView: false, selectedCommunityId: msg.community_id, selectedChannelId: channelId })
      set({ jumpTarget: { channelId, messageId: msg.id } })
      if (loaded) return
      try {
        const page = await endpoints.messagesAt(channelId, 'around', msg.id, PAGE_SIZE)
        const asc = page.slice().reverse()
        const newer = asc.filter((m) => m.id > msg.id).length
        const older = asc.length - newer
        set((s) => ({
          messages: {
            ...s.messages,
            [channelId]: {
              items: asc,
              hasMore: older >= PAGE_SIZE - Math.floor(PAGE_SIZE / 2),
              hasNewer: newer >= Math.floor(PAGE_SIZE / 2),
              loading: false,
            },
          },
        }))
      } catch (e) {
        set((s) => {
          const messages = { ...s.messages }
          delete messages[channelId]
          return { messages, jumpTarget: null }
        })
        throw e
      }
    },

    clearJumpTarget() {
      set({ jumpTarget: null })
    },

    async loadNewer(channelId) {
      const current = get().messages[channelId]
      const last = current?.items[current.items.length - 1]
      if (!current || current.loading || !current.hasNewer || !last) return
      set((s) => ({ messages: { ...s.messages, [channelId]: { ...current, loading: true } } }))
      try {
        const page = await endpoints.messagesAt(channelId, 'after', last.id, PAGE_SIZE)
        set((s) => {
          let items = s.messages[channelId]?.items ?? []
          for (const m of page) items = upsertMessage(items, m)
          return {
            messages: {
              ...s.messages,
              [channelId]: { ...current, items, loading: false, hasNewer: page.length === PAGE_SIZE },
            },
          }
        })
      } catch (e) {
        set((s) => ({ messages: { ...s.messages, [channelId]: { ...current, loading: false } } }))
        throw e
      }
    },

    openSearch(search) {
      set({ search })
    },

    closeSearch() {
      set({ search: null })
    },

    async loadProfile(userId) {
      const profile = await endpoints.userProfile(userId)
      set((s) => ({ profiles: { ...s.profiles, [userId]: profile } }))
    },

    openProfile(userId, communityId, anchor) {
      const { top, left, right, bottom } = anchor
      set({ profileCard: { userId, communityId, anchor: { top, left, right, bottom } } })
    },

    closeProfile() {
      set({ profileCard: null })
    },

    upsertMember(communityId, member) {
      set((s) => {
        const list = s.members[communityId]
        if (!list) return {}
        const exists = list.some((m) => m.user.id === member.user.id)
        return {
          members: {
            ...s.members,
            [communityId]: exists ? list.map((m) => (m.user.id === member.user.id ? member : m)) : [...list, member],
          },
        }
      })
    },
  }
})

/** Anzeigename eines Nutzers. */
export function displayName(u: { username: string; display_name: string | null } | null): string {
  if (!u) return 'Gelöschter Nutzer'
  return u.display_name || u.username
}

/** Absender einer Nachricht: Name des Webhooks oder Anzeigename des Autors. */
export function senderName(m: Pick<Message, 'author' | 'webhook'>): string {
  return m.webhook?.name ?? displayName(m.author)
}

/** Name eines Mitglieds in seiner Community: Nickname > Anzeigename > Benutzername. */
export function memberName(m: Pick<Member, 'user' | 'nickname'>): string {
  return m.nickname || displayName(m.user)
}

/** Name eines Nutzers in einer Community (mit Nickname, falls die Mitgliederliste geladen ist). */
export function useMemberName(
  communityId: string | null | undefined,
  u: { id: string; username: string; display_name: string | null } | null,
): string {
  const nickname = useStore((s) => (communityId && u ? memberOf(s.members[communityId], u.id)?.nickname : null))
  return nickname || displayName(u)
}

/** Mitglied einer Community (undefined, solange die Mitgliederliste nicht geladen ist). */
export function useMember(communityId: string | null | undefined, userId: string | null | undefined): Member | undefined {
  return useStore((s) => (communityId ? memberOf(s.members[communityId], userId) : undefined))
}
