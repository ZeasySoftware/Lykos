/**
 * Läuft der Client aus dem gebauten Desktop-Bundle? Bewusst über den Origin statt `isTauri()`:
 * Fremde Tauri-Hosts (z. B. eingebettete Browser-Panels) setzen die Tauri-Globals ebenfalls.
 */
export const isDesktopBundle = location.protocol === 'tauri:' || location.hostname === 'tauri.localhost'

// ---------------------------------------------------------------------------
// Server-URL (im Desktop-Bundle frei wählbar)
// ---------------------------------------------------------------------------

/** Standard-Instanz des Desktop-Clients. */
export const DEFAULT_SERVER_URL = 'https://lykos.zeasy.dev'
const SERVER_KEY = 'lykos_server_url'

/**
 * Basis-URL des Lykos-Servers.
 * Im Browser und im Tauri-Dev-Modus ist der Client same-origin (Server liefert ihn aus bzw. Vite-Proxy),
 * im Desktop-Bundle muss die Instanz explizit angesprochen werden – gewählt vom Nutzer, sonst Standard.
 */
let serverUrl: string = isDesktopBundle
  ? (safeGet(SERVER_KEY) ?? import.meta.env.VITE_LYKOS_SERVER ?? DEFAULT_SERVER_URL)
  : (import.meta.env.VITE_LYKOS_SERVER ?? '')

export function getServerUrl(): string {
  return serverUrl
}

/** Hat der Nutzer im Desktop-Bundle schon eine Instanz gewählt? (Im Web immer `true`.) */
export function isServerChosen(): boolean {
  return !isDesktopBundle || safeGet(SERVER_KEY) !== null
}

/** URL vereinheitlichen: Schema ergänzen, Pfad/abschließenden Slash entfernen. `null` = ungültig. */
export function normalizeServerUrl(input: string): string | null {
  const raw = input.trim()
  if (!raw) return null
  try {
    const url = new URL(/^[a-z]+:\/\//i.test(raw) ? raw : `https://${raw}`)
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return null
    return url.origin
  } catch {
    return null
  }
}

/**
 * Instanz wechseln (nur Desktop-Bundle). Lädt das Token dieser Instanz aus dem Schlüsselbund;
 * der Aufrufer setzt danach den App-Zustand zurück.
 */
export async function setServerUrl(url: string): Promise<void> {
  serverUrl = url
  safeSet(SERVER_KEY, url)
  await loadToken()
}

/** Wird der Server cross-origin angesprochen? Dann gibt es kein Cookie, nur das Bearer-Token. */
function usesBearer(): boolean {
  return serverUrl !== '' && new URL(serverUrl, location.href).origin !== location.origin
}

// ---------------------------------------------------------------------------
// Token (nur Desktop/cross-origin; im Web trägt das HttpOnly-Cookie die Session)
// ---------------------------------------------------------------------------

const TOKEN_KEY = 'lykos_token'

/**
 * Im Desktop-Bundle liegt das Token im OS-Schlüsselbund (Tauri-Commands `token_*`, ein Eintrag pro
 * Instanz), sonst – nur bei cross-origin, z. B. Dev-Client gegen fremden Server – im localStorage.
 */
let token: string | null = !isDesktopBundle && usesBearer() ? safeGet(TOKEN_KEY) : null

function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function safeSet(key: string, value: string | null) {
  try {
    if (value) localStorage.setItem(key, value)
    else localStorage.removeItem(key)
  } catch {
    // Speicher nicht verfügbar – Wert lebt dann nur im Speicher.
  }
}

async function keyring<T>(cmd: 'token_get' | 'token_set' | 'token_delete', args: Record<string, string>) {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(cmd, args)
}

/** Token aus dem Schlüsselbund laden (Desktop-Bundle; sonst ohne Wirkung). Vor dem ersten Request aufrufen. */
export async function loadToken(): Promise<void> {
  if (!isDesktopBundle) return
  try {
    token = await keyring<string | null>('token_get', { server: serverUrl })
  } catch (e) {
    console.warn('Schlüsselbund nicht lesbar', e)
    token = null
  }
  // Altlast: früher lag das Token im localStorage – einmalig umziehen.
  const legacy = safeGet(TOKEN_KEY)
  if (legacy) {
    safeSet(TOKEN_KEY, null)
    if (!token) setToken(legacy)
  }
}

export function getToken(): string | null {
  return token
}

export function setToken(value: string | null) {
  if (!usesBearer()) return
  token = value
  if (isDesktopBundle) {
    const done = value
      ? keyring<void>('token_set', { server: serverUrl, token: value })
      : keyring<void>('token_delete', { server: serverUrl })
    // Schlüsselbund nicht verfügbar – Token lebt dann nur im Speicher.
    done.catch((e) => console.warn('Schlüsselbund nicht beschreibbar', e))
    return
  }
  safeSet(TOKEN_KEY, value)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string) {
    super(`API-Fehler ${status}: ${code}`)
    this.status = status
    this.code = code
  }
}

type Method = 'GET' | 'POST' | 'PATCH' | 'PUT' | 'DELETE'

export async function api<T>(path: string, method: Method = 'GET', body?: unknown): Promise<T> {
  const headers: Record<string, string> = {}
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  if (token) headers.Authorization = `Bearer ${token}`

  let res: Response
  try {
    res = await fetch(`${serverUrl}/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: usesBearer() ? 'omit' : 'same-origin',
    })
  } catch {
    throw new ApiError(0, 'network')
  }

  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new ApiError(res.status, data.error ?? 'unknown')
  }
  if (res.status === 204) return undefined as T
  const text = await res.text()
  return (text ? JSON.parse(text) : undefined) as T
}

// ---------------------------------------------------------------------------
// Typen (siehe docs/api.md)
// ---------------------------------------------------------------------------

export interface Health {
  status: 'ok' | 'degraded'
  version: string
  database: 'ok' | 'unreachable'
}

export type RegistrationMode = 'open' | 'invite_only' | 'closed'

/** `GET /api/desktop/latest` – neuester Windows-Installer. */
export interface DesktopRelease {
  version: string
  notes: string
  published_at: string | null
  release_url: string
  file_name: string
  download_url: string
  size: number
  sha256: string | null
}

export interface InstanceInfo {
  name: string
  public_url: string
  registration_mode: RegistrationMode
  bootstrap: boolean
  /** Maximale Upload-Größe in Bytes (ältere Server liefern das Feld nicht). */
  max_upload_bytes?: number
  /** Voice/Screenshare verfügbar (LiveKit konfiguriert). */
  voice?: boolean
  /** Dürfen alle Nutzer Communities anlegen (sonst nur Instanz-Admins)? Ältere Server: fehlt = ja. */
  allow_community_creation?: boolean
  /** Ankündigung der Instanz-Admins, die allen angezeigt wird. */
  announcement?: string | null
  /** Mailversand eingerichtet: E-Mail-Adresse, „Passwort vergessen“, 2FA per E-Mail. */
  email?: boolean
  /** Konfigurierte OIDC-Anbieter („Mit … anmelden“). */
  oidc_providers?: OidcProvider[]
  /** Quellcode der laufenden Fassung (AGPL-3.0); ältere Server liefern das Feld nicht. */
  source_url?: string
  /** Hinterlegte Rechtstexte (`/legal/imprint`, `/legal/privacy`); ältere Server liefern das Feld nicht. */
  legal?: LegalAvailability
}

export interface LegalAvailability {
  imprint: boolean
  privacy: boolean
}

export type LegalKind = 'imprint' | 'privacy'

export interface OidcProvider {
  id: string
  label: string
}

export interface User {
  id: string
  username: string
  display_name: string | null
  is_instance_admin: boolean
  created_at: string
  /** Pfad des Profilbilds (`/api/users/{id}/avatar/{avatar_id}`), ältere Server liefern das Feld nicht. */
  avatar_url?: string | null
  /** „Über mich“, max. 190 Zeichen. */
  bio?: string | null
  /** Status-Text, max. 128 Zeichen. */
  status_text?: string | null
  /** Pronomen, max. 40 Zeichen. */
  pronouns?: string | null
  /** Pfad des Profilbanners (`/api/users/{id}/banner/{banner_id}`). */
  banner_url?: string | null
  /** Bannerfarbe 0xRRGGBB (ohne Bannerbild). */
  banner_color?: number | null
  /** Akzentfarbe der Profilkarte 0xRRGGBB. */
  accent_color?: number | null
}

export interface UserPublic {
  id: string
  username: string
  display_name: string | null
  avatar_url?: string | null
}

/** Profil eines Nutzers (`GET /users/{id}`, Gateway `USER_UPDATE`). */
export interface UserProfile extends UserPublic {
  bio: string | null
  status_text: string | null
  created_at: string
  pronouns?: string | null
  banner_url?: string | null
  banner_color?: number | null
  accent_color?: number | null
  /** Nur in `GET /users/{id}`: gemeinsame Communities (fehlt in `USER_UPDATE`). */
  mutual_community_ids?: string[]
}

export interface Session {
  id: string
  label: string | null
  created_at: string
  last_used_at: string
  expires_at: string
  current: boolean
}

/** Speicherbelegung durch Anhänge (`quota_bytes: null` = unbegrenzt). */
export interface StorageUsage {
  used_bytes: number
  quota_bytes: number | null
}

export interface Community {
  id: string
  name: string
  owner_id: string
  message_retention_days: number | null
  /** Speicher-Kontingent der Community für Anhänge in MB; `null` = unbegrenzt. */
  upload_quota_mb: number | null
  created_at: string
  /** `/api/communities/{id}/icon/{icon_id}` (ältere Server liefern die Gestaltungsfelder nicht). */
  icon_url?: string | null
  /** `/api/communities/{id}/banner/{banner_id}`. */
  banner_url?: string | null
  /** Kurzbeschreibung, max. 300 Zeichen. */
  description?: string | null
  /** Akzentfarbe 0xRRGGBB. */
  accent_color?: number | null
  /** Textkanal für Beitritts-Meldungen. */
  system_channel_id?: string | null
}

export type ChannelKind = 'text' | 'voice'

export interface Channel {
  id: string
  community_id: string
  name: string
  kind: ChannelKind
  topic: string | null
  position: number
  /** Ton-Einstellungen (nur bei Sprachkanälen wirksam): Ziel-Bitrate in kbit/s (32–256). */
  audio_bitrate: number
  audio_stereo: boolean
  audio_dtx: boolean
  audio_red: boolean
  /** Höchstzahl gleichzeitiger Teilnehmer (1–99), `null` = unbegrenzt. */
  user_limit: number | null
}

export type DmKind = 'dm' | 'group_dm'

/**
 * Kanal mit Nachrichtenverlauf, wie ihn die Nachrichtenansicht braucht: Textkanal einer Community
 * oder DM (dann `community_id: null`, `name` = Anzeigename der DM).
 */
export type MessageChannel = Pick<Channel, 'id' | 'name' | 'topic' | 'position'> & {
  community_id: string | null
  kind: 'text' | DmKind
}

/** Direktnachrichten-Kanal (1:1 oder Gruppe). */
export interface DmChannel {
  id: string
  kind: DmKind
  /** Nur Gruppen; `null` = Teilnehmer anzeigen. */
  name: string | null
  /** Alle Teilnehmer inkl. einem selbst. */
  recipients: UserPublic[]
  last_message_id: string | null
  /** In Gateway-Events immer `null`. */
  last_read_message_id: string | null
  /** Ungelesene Erwähnungen (in Gateway-Events immer 0). */
  mention_count: number
}

/** Beziehung aus eigener Sicht (Freundesystem). */
export type RelationshipType = 'friend' | 'pending_out' | 'pending_in' | 'blocked'

export interface Relationship {
  /** ID des anderen Nutzers. */
  id: string
  type: RelationshipType
  user: UserPublic
  /** Seit wann die Beziehung in dieser Form besteht. */
  since: string
}

/** Wer darf einem Freundschaftsanfragen schicken? */
export type FriendRequestPolicy = 'everyone' | 'mutual_communities' | 'nobody'

export interface Role {
  id: string
  community_id: string
  name: string
  permissions: number
  position: number
  color: number | null
  is_default: boolean
}

export interface Member {
  user: UserPublic
  nickname: string | null
  role_ids: string[]
  joined_at: string
}

export interface Message {
  id: string
  channel_id: string
  /** `null` in DMs. */
  community_id: string | null
  author: UserPublic | null
  content: string
  created_at: string
  edited_at: string | null
  attachments: Attachment[]
  /** Antwort auf diese Nachricht (bleibt gesetzt, wenn das Original gelöscht wurde). */
  reply_to_id?: string | null
  /** Vorschau des Originals; `null` = gelöscht. */
  referenced_message?: MessageReference | null
  reactions?: Reaction[]
  /** IDs erwähnter Nutzer. */
  mentions?: string[]
  mention_everyone?: boolean
  pinned_at?: string | null
  /**
   * Systemnachrichten (Inhalt leer): `member_join` = „… ist beigetreten“, `call_missed` = nicht angenommener
   * Anruf in einer 1:1-DM (Autor = Anrufer). Fehlt bei älteren Servern.
   */
  kind?: 'default' | 'member_join' | 'call_missed'
}

export interface MessageReference {
  id: string
  author: UserPublic | null
  /** Auf 200 Zeichen gekürzt. */
  content: string
  has_attachments: boolean
}

export interface Reaction {
  /** Unicode-Emoji oder bei Custom-Emotes `name:id`. */
  emoji: string
  count: number
  /** Habe ich so reagiert? */
  me: boolean
  /** ID des Custom-Emotes (ältere Server liefern das Feld nicht). */
  emoji_id?: string | null
  animated?: boolean
}

/** Custom-Emote einer Community (`/emojis`, `/communities/{id}/emojis`, Gateway `EMOJIS_UPDATE`). */
export interface CustomEmoji {
  id: string
  community_id: string
  name: string
  animated: boolean
  /** Pfad des Bilds, z. B. `/api/emojis/{id}.png`. */
  url: string
  uploader: UserPublic | null
  created_at: string
}

export interface Invite {
  code: string
  community_id: string
  max_uses: number | null
  uses: number
  expires_at: string | null
  created_at: string
}

export interface InvitePreview {
  code: string
  /** `instance` = Registrierungs-Einladung der Instanz (ältere Server liefern das Feld nicht). */
  kind?: 'community' | 'instance'
  instance_name?: string
  /** Bei Instanz-Einladungen ohne automatischen Community-Beitritt `null`. */
  community: {
    id: string
    name: string
    /** Bild-URLs über den Einladungscode (öffentlich abrufbar, solange die Einladung gilt). */
    icon_url?: string | null
    banner_url?: string | null
    description?: string | null
    accent_color?: number | null
  } | null
  member_count: number
}

export interface CommunityDetail {
  community: Community
  channels: Channel[]
  roles: Role[]
  permissions: number
  channel_permissions: Record<string, number>
  /** Lesestand je sichtbarem Textkanal (channel_id → last_read_message_id). */
  read_states: Record<string, string>
  /** Neueste Nachricht je sichtbarem Textkanal (channel_id → message_id). */
  last_message_ids: Record<string, string>
  /** Ungelesene Erwähnungen je sichtbarem Textkanal (nur Kanäle mit mindestens einer). */
  mention_counts?: Record<string, number>
}

/** Sichtbarer Status eines Nutzers. */
export type PresenceStatus = 'online' | 'idle' | 'dnd' | 'offline'
/** Selbst gewählter Status (`invisible` = für andere offline). */
export type ChosenStatus = 'online' | 'idle' | 'dnd' | 'invisible'
export interface VoiceState {
  /** `null` bei Anrufen in DMs. */
  community_id: string | null
  channel_id: string
  user_id: string
  self_mute: boolean
  self_deaf: boolean
  streaming: boolean
  /** Kamera ist an. */
  video: boolean
  joined_at?: string
}

/** Klingelzustand eines DM-Anrufs (`CALL_UPDATE`). */
export interface CallUpdate {
  channel_id: string
  /** Bei wem es gerade klingelt. */
  ringing: string[]
  /** Wer angerufen hat; `null`, wenn es nirgends klingelt. */
  caller_id: string | null
}

/** Laufender Anruf einer eigenen DM (READY/RESYNC). */
export interface DmCall extends CallUpdate {
  voice_states: VoiceState[]
}

export interface VoiceJoin {
  /** Öffentliche LiveKit-URL (wss://…). */
  url: string
  /** Kurzlebiges LiveKit-Access-Token. */
  token: string
  /** Ton-Einstellungen des Kanals für die eigene Mikrofonspur. */
  audio: { bitrate: number; stereo: boolean; dtx: boolean; red: boolean }
}

export interface AuthResponse {
  /** Fehlt im eigenen Web-Client (Sitzung nur per HttpOnly-Cookie); Desktop/Bearer bekommt es. */
  token?: string
  user: User
}

// --- Konto-Sicherheit (Paket G) ---

export type MfaMethod = 'totp' | 'email' | 'recovery'

/** 2FA aktiv: statt einer Session ein kurzlebiges Token für den zweiten Schritt. */
export interface MfaChallenge {
  mfa_required: true
  mfa_token: string
  methods: MfaMethod[]
}

export type LoginResult = AuthResponse | MfaChallenge

export function isMfaChallenge(r: LoginResult): r is MfaChallenge {
  return 'mfa_required' in r && r.mfa_required === true
}

export interface MfaStatus {
  totp: boolean
  email: boolean
  recovery_codes_left: number
}

export interface LinkedIdentity {
  provider: string
  /** `null`, wenn der Anbieter nicht mehr konfiguriert ist. */
  label: string | null
  created_at: string
  last_used_at: string | null
}

export interface SecurityOverview {
  email: string | null
  pending_email: string | null
  has_password: boolean
  mfa: MfaStatus
  identities: LinkedIdentity[]
}

/** Bestätigung vor sensiblen Aktionen: Passwort und ggf. 2FA-Code. */
export interface Reauth {
  password?: string
  mfa_code?: string
  mfa_method?: MfaMethod
}

export interface TotpSetup {
  secret: string
  otpauth_uri: string
  qr_svg: string
}

export interface RecoveryCodes {
  recovery_codes: string[] | null
}

export interface OidcSignupInfo {
  provider: string
  provider_label: string
  email: string | null
}

// --- Verwaltung (Paket A) ---

export interface AuditEntry {
  id: string
  actor: UserPublic | null
  action: string
  target_id: string | null
  data: Record<string, unknown>
  created_at: string
}

export interface ChannelOverride {
  role_id: string
  allow: number
  deny: number
}

// ---------------------------------------------------------------------------
// Endpunkte
// ---------------------------------------------------------------------------

const sessionLabel = isDesktopBundle ? 'Desktop-App' : 'Web-Browser'
const enc = encodeURIComponent

export const endpoints = {
  health: () => api<Health>('/health'),
  instance: () => api<InstanceInfo>('/instance'),
  /** Neuester Desktop-Installer (öffentlich, Download-Seite). */
  desktopLatest: () => api<DesktopRelease>('/desktop/latest'),
  legal: (kind: LegalKind) => api<{ content: string }>(`/legal/${kind}`),

  register: (body: { username: string; password: string; display_name?: string; invite?: string; email?: string }) =>
    api<AuthResponse>('/auth/register', 'POST', { ...body, session_label: sessionLabel }),
  login: (body: { username: string; password: string }) =>
    api<LoginResult>('/auth/login', 'POST', { ...body, session_label: sessionLabel }),
  loginMfa: (body: { mfa_token: string; method: MfaMethod; code: string }) =>
    api<AuthResponse>('/auth/login/mfa', 'POST', body),
  loginMfaEmail: (mfa_token: string) => api<void>('/auth/login/mfa/email', 'POST', { mfa_token }),

  // Konto-Sicherheit
  security: () => api<SecurityOverview>('/auth/security'),
  setEmail: (body: { email: string } & Reauth) => api<{ pending_email: string }>('/auth/email', 'PUT', body),
  deleteEmail: (body: Reauth) => api<void>('/auth/email', 'DELETE', body),
  verifyEmail: (token: string) => api<{ email: string }>('/auth/email/verify', 'POST', { token }),
  forgotPassword: (login: string) => api<void>('/auth/password/forgot', 'POST', { login }),
  totpSetup: (body: Reauth) => api<TotpSetup>('/auth/mfa/totp/setup', 'POST', body),
  totpEnable: (code: string) => api<RecoveryCodes>('/auth/mfa/totp/enable', 'POST', { code }),
  totpDisable: (body: Reauth) => api<void>('/auth/mfa/totp', 'DELETE', body),
  emailMfaEnable: (body: Reauth) => api<RecoveryCodes>('/auth/mfa/email', 'POST', body),
  emailMfaDisable: (body: Reauth) => api<void>('/auth/mfa/email', 'DELETE', body),
  /** Code per E-Mail, um eine sensible Aktion zu bestätigen. */
  sendReauthCode: () => api<void>('/auth/mfa/email-code', 'POST'),
  regenerateRecoveryCodes: (body: Reauth) => api<RecoveryCodes>('/auth/mfa/recovery-codes', 'POST', body),
  oidcStart: (provider: string, mode: 'login') =>
    api<{ url: string }>(`/auth/oidc/${enc(provider)}/start`, 'POST', { mode, session_label: sessionLabel }),
  /** Verknüpfen braucht wie das Trennen eine Bestätigung (Passwort bzw. frische Anmeldung, ggf. 2FA). */
  oidcLinkStart: (provider: string, body: Reauth) =>
    api<{ url: string }>(`/auth/oidc/${enc(provider)}/start`, 'POST', { ...body, mode: 'link' }),
  oidcSignupInfo: (signup_token: string) => api<OidcSignupInfo>('/auth/oidc/signup', 'POST', { signup_token }),
  oidcRegister: (body: {
    signup_token: string
    username: string
    display_name?: string
    invite?: string
    use_email: boolean
  }) => api<AuthResponse>('/auth/oidc/register', 'POST', body),
  unlinkIdentity: (provider: string, body: Reauth) => api<void>(`/auth/identities/${enc(provider)}`, 'DELETE', body),
  logout: () => api<void>('/auth/logout', 'POST'),
  me: () => api<User>('/auth/me'),
  /** Eigene Speicherbelegung durch Anhänge. */
  myUsage: () => api<StorageUsage>('/auth/me/usage'),
  communityUsage: (id: string) => api<StorageUsage>(`/communities/${enc(id)}/usage`),
  updateMe: (body: {
    display_name?: string | null
    bio?: string | null
    status_text?: string | null
    pronouns?: string | null
    banner_color?: number | null
    accent_color?: number | null
  }) => api<User>('/auth/me', 'PATCH', body),
  deleteProfileBanner: () => api<User>('/auth/me/banner', 'DELETE'),
  sessions: () => api<Session[]>('/auth/sessions'),
  revokeSession: (id: string) => api<void>(`/auth/sessions/${enc(id)}`, 'DELETE'),
  revokeOtherSessions: () => api<void>('/auth/sessions', 'DELETE'),
  changePassword: (body: {
    current_password?: string
    new_password: string
    logout_other_sessions: boolean
    mfa_code?: string
    mfa_method?: MfaMethod
  }) => api<void>('/auth/password', 'POST', body),
  deleteAccount: (body: Reauth & { delete_messages: boolean }) => api<void>('/auth/me', 'DELETE', body),
  deleteAvatar: () => api<User>('/auth/me/avatar', 'DELETE'),
  userProfile: (id: string) => api<UserProfile>(`/users/${enc(id)}`),

  communities: () => api<Community[]>('/communities'),
  createCommunity: (name: string) => api<CommunityDetail>('/communities', 'POST', { name }),
  community: (id: string) => api<CommunityDetail>(`/communities/${enc(id)}`),
  updateCommunity: (
    id: string,
    body: {
      name?: string
      message_retention_days?: number | null
      upload_quota_mb?: number | null
      description?: string | null
      accent_color?: number | null
      system_channel_id?: string | null
    },
  ) =>
    api<Community>(`/communities/${enc(id)}`, 'PATCH', body),
  deleteCommunityImage: (id: string, kind: 'icon' | 'banner') =>
    api<Community>(`/communities/${enc(id)}/${kind}`, 'DELETE'),
  members: (id: string) => api<Member[]>(`/communities/${enc(id)}/members`),
  removeMember: (communityId: string, userId: string) =>
    api<void>(`/communities/${enc(communityId)}/members/${enc(userId)}`, 'DELETE'),
  /** Nickname setzen; `userId` = `@me` für den eigenen. `null` entfernt ihn. */
  setNickname: (communityId: string, userId: string, nickname: string | null) =>
    api<Member>(`/communities/${enc(communityId)}/members/${enc(userId)}`, 'PATCH', { nickname }),
  transferCommunity: (communityId: string, userId: string) =>
    api<Community>(`/communities/${enc(communityId)}/transfer`, 'POST', { user_id: userId }),

  createChannel: (communityId: string, body: { name: string; kind: ChannelKind; topic?: string }) =>
    api<Channel>(`/communities/${enc(communityId)}/channels`, 'POST', body),

  messages: (channelId: string, before?: string, limit = 50) => {
    const q = new URLSearchParams({ limit: String(limit) })
    if (before) q.set('before', before)
    return api<Message[]>(`/channels/${enc(channelId)}/messages?${q}`)
  },
  /** Nachrichten um eine ID herum (`around`) bzw. neuere als eine ID (`after`), neueste zuerst. */
  messagesAt: (channelId: string, mode: 'around' | 'after', messageId: string, limit = 50) => {
    const q = new URLSearchParams({ limit: String(limit), [mode]: messageId })
    return api<Message[]>(`/channels/${enc(channelId)}/messages?${q}`)
  },
  sendMessage: (channelId: string, content: string, attachmentIds: string[] = [], replyToId?: string) =>
    api<Message>(`/channels/${enc(channelId)}/messages`, 'POST', {
      content,
      attachment_ids: attachmentIds,
      reply_to_id: replyToId,
    }),
  editMessage: (id: string, content: string) => api<Message>(`/messages/${enc(id)}`, 'PATCH', { content }),
  deleteMessage: (id: string) => api<void>(`/messages/${enc(id)}`, 'DELETE'),
  addReaction: (messageId: string, emoji: string) =>
    api<void>(`/messages/${enc(messageId)}/reactions/${enc(emoji)}/@me`, 'PUT'),
  removeReaction: (messageId: string, emoji: string) =>
    api<void>(`/messages/${enc(messageId)}/reactions/${enc(emoji)}/@me`, 'DELETE'),
  pins: (channelId: string) => api<Message[]>(`/channels/${enc(channelId)}/pins`),
  pin: (channelId: string, messageId: string) =>
    api<void>(`/channels/${enc(channelId)}/pins/${enc(messageId)}`, 'PUT'),
  unpin: (channelId: string, messageId: string) =>
    api<void>(`/channels/${enc(channelId)}/pins/${enc(messageId)}`, 'DELETE'),
  markRead: (channelId: string, messageId: string) =>
    api<void>(`/channels/${enc(channelId)}/read`, 'PUT', { message_id: messageId }),

  createInvite: (communityId: string, body: { max_uses?: number; expires_in_hours?: number }) =>
    api<Invite>(`/communities/${enc(communityId)}/invites`, 'POST', body),
  invite: (code: string) => api<InvitePreview>(`/invites/${enc(code)}`),
  acceptInvite: (code: string) => api<CommunityDetail>(`/invites/${enc(code)}/accept`, 'POST'),

  // --- Verwaltung (Paket A) ---
  deleteCommunity: (id: string) => api<void>(`/communities/${enc(id)}`, 'DELETE'),
  addMemberRole: (communityId: string, userId: string, roleId: string) =>
    api<void>(`/communities/${enc(communityId)}/members/${enc(userId)}/roles/${enc(roleId)}`, 'PUT'),
  removeMemberRole: (communityId: string, userId: string, roleId: string) =>
    api<void>(`/communities/${enc(communityId)}/members/${enc(userId)}/roles/${enc(roleId)}`, 'DELETE'),
  auditLog: (communityId: string, before?: string, limit = 50) => {
    const q = new URLSearchParams({ limit: String(limit) })
    if (before) q.set('before', before)
    return api<AuditEntry[]>(`/communities/${enc(communityId)}/audit-log?${q}`)
  },
  createRole: (communityId: string, body: { name: string; permissions?: number; color?: number | null }) =>
    api<Role>(`/communities/${enc(communityId)}/roles`, 'POST', body),
  updateRole: (id: string, body: { name?: string; permissions?: number; color?: number | null; position?: number }) =>
    api<Role>(`/roles/${enc(id)}`, 'PATCH', body),
  deleteRole: (id: string) => api<void>(`/roles/${enc(id)}`, 'DELETE'),
  /** Reihenfolge aller Rollen außer „@alle“ setzen (höchste zuerst). */
  reorderRoles: (communityId: string, roleIds: string[]) =>
    api<Role[]>(`/communities/${enc(communityId)}/roles/order`, 'PATCH', { role_ids: roleIds }),
  updateChannel: (
    id: string,
    body: {
      name?: string
      topic?: string | null
      position?: number
      audio_bitrate?: number
      audio_stereo?: boolean
      audio_dtx?: boolean
      audio_red?: boolean
      user_limit?: number | null
    },
  ) =>
    api<Channel>(`/channels/${enc(id)}`, 'PATCH', body),
  deleteChannel: (id: string) => api<void>(`/channels/${enc(id)}`, 'DELETE'),
  channelOverrides: (channelId: string) => api<ChannelOverride[]>(`/channels/${enc(channelId)}/overrides`),
  putOverride: (channelId: string, roleId: string, body: { allow: number; deny: number }) =>
    api<void>(`/channels/${enc(channelId)}/overrides/${enc(roleId)}`, 'PUT', body),
  deleteOverride: (channelId: string, roleId: string) =>
    api<void>(`/channels/${enc(channelId)}/overrides/${enc(roleId)}`, 'DELETE'),
  invites: (communityId: string) => api<Invite[]>(`/communities/${enc(communityId)}/invites`),
  deleteInvite: (code: string) => api<void>(`/invites/${enc(code)}`, 'DELETE'),

  // --- Direktnachrichten & Suche ---
  dms: () => api<DmChannel[]>('/dms'),
  createDm: (recipientIds: string[], name?: string) =>
    api<DmChannel>('/dms', 'POST', { recipient_ids: recipientIds, name: name || undefined }),
  renameDm: (id: string, name: string | null) => api<DmChannel>(`/dms/${enc(id)}`, 'PATCH', { name }),
  leaveDm: (id: string) => api<void>(`/dms/${enc(id)}`, 'DELETE'),
  dmContacts: () => api<UserPublic[]>('/dms/contacts'),
  dmSettings: () => api<{ allow_dms: boolean }>('/dms/settings'),
  updateDmSettings: (allowDms: boolean) => api<{ allow_dms: boolean }>('/dms/settings', 'PATCH', { allow_dms: allowDms }),

  // --- Freundesystem ---
  relationships: () => api<Relationship[]>('/relationships'),
  /** Freundschaftsanfrage per Benutzername (nimmt eine offene Gegenanfrage an). */
  requestFriend: (username: string) => api<Relationship>('/relationships', 'POST', { username }),
  /** Anfrage senden bzw. annehmen (`friend`) oder blockieren (`blocked`). */
  putRelationship: (userId: string, type: 'friend' | 'blocked') =>
    api<Relationship>(`/relationships/${enc(userId)}`, 'PUT', { type }),
  /** Freund entfernen, Anfrage ablehnen/zurückziehen oder Blockade aufheben. */
  removeRelationship: (userId: string) => api<void>(`/relationships/${enc(userId)}`, 'DELETE'),
  relationshipSettings: () => api<{ friend_requests: FriendRequestPolicy }>('/relationships/settings'),
  updateRelationshipSettings: (friend_requests: FriendRequestPolicy) =>
    api<{ friend_requests: FriendRequestPolicy }>('/relationships/settings', 'PATCH', { friend_requests }),

  /** Suche in einer Community (`communityId`) oder in den eigenen DMs (`null`). */
  search: (communityId: string | null, query: { q: string; channel_id?: string; before?: string; limit?: number }) => {
    const q = new URLSearchParams({ q: query.q, limit: String(query.limit ?? 25) })
    if (query.channel_id) q.set('channel_id', query.channel_id)
    if (query.before) q.set('before', query.before)
    const base = communityId ? `/communities/${enc(communityId)}/search` : '/dms/search'
    return api<Message[]>(`${base}?${q}`)
  },

  // --- Custom-Emotes (Paket I) ---
  /** Emotes aller eigenen Communities (überall verwendbar). */
  myEmojis: () => api<CustomEmoji[]>('/emojis'),
  communityEmojis: (communityId: string) => api<CustomEmoji[]>(`/communities/${enc(communityId)}/emojis`),
  renameEmoji: (communityId: string, emojiId: string, name: string) =>
    api<CustomEmoji>(`/communities/${enc(communityId)}/emojis/${enc(emojiId)}`, 'PATCH', { name }),
  deleteEmoji: (communityId: string, emojiId: string) =>
    api<void>(`/communities/${enc(communityId)}/emojis/${enc(emojiId)}`, 'DELETE'),

  // --- Voice (Paket B) ---
  voiceJoin: (channelId: string) => api<VoiceJoin>(`/channels/${enc(channelId)}/voice/join`, 'POST'),
  setVoiceState: (body: { self_mute: boolean; self_deaf: boolean }) => api<void>('/voice/state', 'POST', body),
  voiceStates: (communityId: string) => api<VoiceState[]>(`/communities/${enc(communityId)}/voice-states`),

  // --- Anrufe in DMs (Beitritt über voiceJoin) ---
  /** Klingeln für sich beenden bzw. als Anrufer ohne Gegenüber den Anruf abbrechen. */
  callDecline: (channelId: string) => api<void>(`/channels/${enc(channelId)}/call/decline`, 'POST'),
  /** Im Anruf: bei allen, die noch nicht drin sind, erneut klingeln lassen. */
  callRing: (channelId: string) => api<void>(`/channels/${enc(channelId)}/call/ring`, 'POST'),
}

// ---------------------------------------------------------------------------
// Dateianhänge
// ---------------------------------------------------------------------------

export interface Attachment {
  id: string
  channel_id: string
  message_id: string | null
  filename: string
  content_type: string
  size_bytes: number
  width: number | null
  height: number | null
  created_at: string
  /** Pfad relativ zur Instanz, z. B. `/api/attachments/{id}/{filename}`. */
  url: string
}

/** Bildtypen, die der Server inline ausliefert (siehe docs/api.md). */
export const INLINE_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

/**
 * Müssen Anhänge per `fetch` mit Bearer-Token geladen werden? Im Desktop-Bundle gibt es kein Cookie,
 * dort funktionieren `<img src>` und Download-Links nicht direkt.
 */
export function attachmentsNeedToken(): boolean {
  return usesBearer()
}

/** Absolute URL eines Anhangs. */
export function attachmentUrl(a: Pick<Attachment, 'url'>): string {
  return `${serverUrl}${a.url}`
}

/** Anhang als Blob laden (mit Token, falls nötig). */
export async function fetchAttachment(a: Pick<Attachment, 'url'>, signal?: AbortSignal): Promise<Blob> {
  const headers: Record<string, string> = {}
  if (token) headers.Authorization = `Bearer ${token}`
  let res: Response
  try {
    res = await fetch(attachmentUrl(a), { headers, signal, credentials: usesBearer() ? 'omit' : 'same-origin' })
  } catch (e) {
    if (e instanceof DOMException && e.name === 'AbortError') throw e
    throw new ApiError(0, 'network')
  }
  if (!res.ok) throw new ApiError(res.status, 'not_found')
  return res.blob()
}

export interface UploadHandle {
  promise: Promise<Attachment>
  abort: () => void
}

/** Lädt eine Datei hoch (XMLHttpRequest, damit der Fortschritt messbar ist). */
export function uploadAttachment(channelId: string, file: File, onProgress: (fraction: number) => void): UploadHandle {
  const xhr = new XMLHttpRequest()
  const promise = new Promise<Attachment>((resolve, reject) => {
    xhr.open('POST', `${serverUrl}/api/channels/${encodeURIComponent(channelId)}/attachments`)
    if (token) xhr.setRequestHeader('Authorization', `Bearer ${token}`)
    xhr.withCredentials = false
    xhr.responseType = 'json'
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total)
    }
    xhr.onload = () => {
      if (xhr.status === 201) resolve(xhr.response as Attachment)
      else {
        const code = (xhr.response as { error?: string } | null)?.error
        reject(new ApiError(xhr.status, code ?? (xhr.status === 413 ? 'file_too_large' : 'unknown')))
      }
    }
    xhr.onerror = () => reject(new ApiError(0, 'network'))
    xhr.onabort = () => reject(new ApiError(0, 'aborted'))
    const form = new FormData()
    form.append('file', file, file.name)
    xhr.send(form)
  })
  return { promise, abort: () => xhr.abort() }
}

/** Profilbild hochladen (bereits zugeschnitten, siehe `lib/avatar.ts`). */
export function uploadAvatar(file: Blob, filename: string): Promise<User> {
  return uploadImage<User>('/auth/me/avatar', file, filename)
}

/** Profilbanner hochladen (bereits zugeschnitten, siehe `lib/image.ts`). */
export function uploadProfileBanner(file: Blob, filename: string): Promise<User> {
  return uploadImage<User>('/auth/me/banner', file, filename)
}

/** Community-Icon bzw. -Banner hochladen (bereits zugeschnitten). */
export function uploadCommunityImage(
  communityId: string,
  kind: 'icon' | 'banner',
  file: Blob,
  filename: string,
): Promise<Community> {
  return uploadImage<Community>(`/communities/${encodeURIComponent(communityId)}/${kind}`, file, filename)
}

/** Bild per Multipart (Feld `file`) an `/api{path}` senden. */
async function uploadImage<T>(path: string, file: Blob, filename: string): Promise<T> {
  const form = new FormData()
  form.append('file', file, filename)
  const headers: Record<string, string> = {}
  if (token) headers.Authorization = `Bearer ${token}`
  let res: Response
  try {
    res = await fetch(`${serverUrl}/api${path}`, {
      method: 'POST',
      headers,
      body: form,
      credentials: usesBearer() ? 'omit' : 'same-origin',
    })
  } catch {
    throw new ApiError(0, 'network')
  }
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new ApiError(res.status, data.error ?? (res.status === 413 ? 'file_too_large' : 'unknown'))
  }
  return (await res.json()) as T
}

/** Custom-Emote hochladen (statische Bilder vorher verkleinert, siehe `lib/emojiUpload.ts`). */
export async function uploadEmoji(communityId: string, name: string, file: Blob, filename: string): Promise<CustomEmoji> {
  const form = new FormData()
  form.append('name', name)
  form.append('file', file, filename)
  const headers: Record<string, string> = {}
  if (token) headers.Authorization = `Bearer ${token}`
  let res: Response
  try {
    res = await fetch(`${serverUrl}/api/communities/${encodeURIComponent(communityId)}/emojis`, {
      method: 'POST',
      headers,
      body: form,
      credentials: usesBearer() ? 'omit' : 'same-origin',
    })
  } catch {
    throw new ApiError(0, 'network')
  }
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new ApiError(res.status, data.error ?? (res.status === 413 ? 'file_too_large' : 'unknown'))
  }
  return (await res.json()) as CustomEmoji
}

/** Datenexport als Blob (JSON) laden. */
export function fetchExport(): Promise<Blob> {
  return fetchAttachment({ url: '/api/auth/me/export' })
}

/** Noch nicht gesendeten Upload verwerfen. */
export function deleteAttachment(id: string): Promise<void> {
  return api<void>(`/attachments/${encodeURIComponent(id)}`, 'DELETE')
}
