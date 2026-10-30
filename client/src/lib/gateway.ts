import { getServerUrl, getToken } from './api'
import type {
  CallUpdate,
  Category,
  Channel,
  ChosenStatus,
  Community,
  CustomEmoji,
  DmCall,
  DmChannel,
  InstanceInfo,
  Member,
  Message,
  PresenceStatus,
  Relationship,
  Role,
  User,
  UserProfile,
  VoiceState,
} from './api'
import type { Feedback } from './feedbackApi'

/** Server → Client-Events (siehe docs/api.md). */
export type GatewayEvent =
  | {
      t: 'READY'
      d: {
        user: User
        community_ids: string[]
        dm_channel_ids?: string[]
        presences: { user_id: string; status: PresenceStatus }[]
        /** Laufende Anrufe der eigenen DMs (fehlt bei älteren Servern). */
        dm_calls?: DmCall[]
      }
    }
  | { t: 'MESSAGE_CREATE' | 'MESSAGE_UPDATE'; d: Message }
  | { t: 'MESSAGE_DELETE'; d: { id: string; channel_id: string; community_id: string | null } }
  | {
      t: 'MESSAGE_REACTION_ADD' | 'MESSAGE_REACTION_REMOVE'
      d: {
        message_id: string
        channel_id: string
        community_id: string | null
        user_id: string
        /** Unicode-Emoji oder `name:id` (Custom-Emote). */
        emoji: string
        emoji_id?: string | null
        animated?: boolean
      }
    }
  | { t: 'EMOJIS_UPDATE'; d: { community_id: string; emojis: CustomEmoji[] } }
  | { t: 'DM_CREATE' | 'DM_UPDATE'; d: DmChannel }
  | { t: 'DM_DELETE'; d: { id: string } }
  /** Eigene Unterhaltung angepinnt bzw. gelöst (nur an die eigenen Verbindungen). */
  | { t: 'DM_PIN_UPDATE'; d: { channel_id: string; pinned_at: string | null } }
  | { t: 'CHANNEL_CREATE' | 'CHANNEL_UPDATE'; d: Channel }
  | { t: 'CHANNEL_DELETE'; d: { id: string; community_id: string } }
  | { t: 'CATEGORY_CREATE' | 'CATEGORY_UPDATE'; d: Category }
  | { t: 'CATEGORY_DELETE'; d: { id: string; community_id: string } }
  | { t: 'COMMUNITY_UPDATE'; d: Community }
  | { t: 'COMMUNITY_DELETE'; d: { id: string } }
  | { t: 'MEMBER_ADD' | 'MEMBER_UPDATE'; d: { community_id: string; member: Member } }
  | { t: 'MEMBER_REMOVE'; d: { community_id: string; user_id: string } }
  | { t: 'USER_UPDATE'; d: UserProfile }
  | { t: 'ROLE_CREATE' | 'ROLE_UPDATE'; d: Role }
  | { t: 'ROLE_DELETE'; d: { id: string; community_id: string } }
  | { t: 'PERMISSIONS_CHANGED'; d: { community_id: string } }
  | { t: 'VOICE_STATE_UPDATE'; d: VoiceState }
  | { t: 'VOICE_STATE_DELETE'; d: { community_id: string | null; channel_id: string; user_id: string } }
  | { t: 'CALL_UPDATE'; d: CallUpdate }
  | { t: 'RESYNC'; d: { presences?: { user_id: string; status: PresenceStatus }[]; dm_calls?: DmCall[] } }
  | { t: 'PONG'; d: Record<string, never> }
  | { t: 'PRESENCE_UPDATE'; d: { user_id: string; status: PresenceStatus } }
  /** Nach eigenem Beitritt: Status der (nicht-offline) Mitglieder, in den Stand einmischen. */
  | { t: 'PRESENCE_BULK'; d: { presences: { user_id: string; status: PresenceStatus }[] } }
  | { t: 'TYPING_START'; d: { channel_id: string; community_id: string | null; user_id: string } }
  | { t: 'READ_STATE_UPDATE'; d: { channel_id: string; community_id: string | null; last_read_message_id: string } }
  | { t: 'RELATIONSHIP_ADD'; d: Relationship }
  | { t: 'RELATIONSHIP_REMOVE'; d: { id: string } }
  | { t: 'FEEDBACK_UPDATE'; d: Feedback }
  | { t: 'FEEDBACK_DELETE'; d: { id: string } }
  | { t: 'INSTANCE_UPDATE'; d: InstanceInfo }
  | { t: 'INSTANCE_ADMIN_UPDATE'; d: { is_instance_admin: boolean } }

/** Client → Server-Ops (außer identify/ping, die die Verbindung selbst sendet). */
export type GatewayOp = { op: 'presence'; status: ChosenStatus } | { op: 'typing'; channel_id: string }

export type GatewayStatus = 'connecting' | 'online' | 'offline'

/** Close-Code, mit dem der Server eine widerrufene Session beendet. */
const CLOSE_SESSION_REVOKED = 4001
const PING_INTERVAL_MS = 30_000
const MAX_BACKOFF_MS = 30_000

interface Handlers {
  onEvent: (event: GatewayEvent) => void
  onStatus: (status: GatewayStatus) => void
  /** Session ungültig/widerrufen – Client soll abmelden. */
  onUnauthorized: () => void
  /** Status, mit dem sich die Verbindung anmeldet (z. B. unsichtbar, ohne kurz online zu erscheinen). */
  initialStatus?: () => ChosenStatus
}

function gatewayUrl(status?: ChosenStatus): string {
  const base = new URL(getServerUrl() || location.origin, location.href)
  base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:'
  base.pathname = '/api/gateway'
  if (status) base.search = `?status=${status}`
  return base.toString()
}

/** WebSocket-Verbindung mit automatischem Reconnect (exponentielles Backoff mit Jitter). */
export class Gateway {
  private ws: WebSocket | null = null
  private attempts = 0
  private stopped = false
  private reconnectTimer: number | undefined
  private pingTimer: number | undefined
  private readonly handlers: Handlers

  constructor(handlers: Handlers) {
    this.handlers = handlers
  }

  start() {
    this.stopped = false
    this.connect()
  }

  stop() {
    this.stopped = true
    window.clearTimeout(this.reconnectTimer)
    window.clearInterval(this.pingTimer)
    this.ws?.close(1000)
    this.ws = null
  }

  /** Op senden, falls verbunden (sonst verworfen – der Status wird beim Verbinden ohnehin übertragen). */
  send(op: GatewayOp) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(op))
  }

  private connect() {
    this.handlers.onStatus('connecting')
    const ws = new WebSocket(gatewayUrl(this.handlers.initialStatus?.()))
    this.ws = ws

    ws.onopen = () => {
      const token = getToken()
      if (token) ws.send(JSON.stringify({ op: 'identify', token }))
      window.clearInterval(this.pingTimer)
      this.pingTimer = window.setInterval(() => {
        if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ op: 'ping' }))
      }, PING_INTERVAL_MS)
    }

    ws.onmessage = (msg) => {
      let event: GatewayEvent
      try {
        event = JSON.parse(String(msg.data)) as GatewayEvent
      } catch {
        return
      }
      if (event.t === 'READY') {
        this.attempts = 0
        this.handlers.onStatus('online')
      }
      this.handlers.onEvent(event)
    }

    ws.onclose = (e) => {
      window.clearInterval(this.pingTimer)
      if (this.ws !== ws) return
      this.ws = null
      this.handlers.onStatus('offline')
      if (this.stopped) return
      if (e.code === CLOSE_SESSION_REVOKED) {
        this.stopped = true
        this.handlers.onUnauthorized()
        return
      }
      this.scheduleReconnect()
    }
  }

  private scheduleReconnect() {
    const delay = Math.min(MAX_BACKOFF_MS, 1000 * 2 ** this.attempts) * (0.75 + Math.random() * 0.5)
    this.attempts += 1
    this.reconnectTimer = window.setTimeout(() => {
      if (!this.stopped) this.connect()
    }, delay)
  }
}
