/**
 * Instanz-Verwaltung (Paket H): Typen und Endpunkte unter `/api/admin/*` (nur Instanz-Admins) sowie das
 * Einlösen von Passwort-Reset-Links. Siehe docs/api.md, Abschnitt „Instanz-Verwaltung“.
 */
import { api } from './api'
import { linkToken } from './authFragment'
import type { AuditEntry, Community, RegistrationMode, UserPublic } from './api'

const enc = encodeURIComponent

export interface Page<T> {
  total: number
  items: T[]
}

export interface AdminOverview {
  users: { total: number; new_7d: number; disabled: number; admins: number }
  communities: number
  messages: number
  storage: {
    total_bytes: number
    attachment_count: number
    backend: 'local' | 's3'
    top_communities: { id: string; name: string; bytes: number }[]
  }
  online: number
  voice: {
    enabled: boolean
    participants: number
    channels: {
      community_id: string
      community_name: string
      channel_id: string
      channel_name: string
      participants: number
    }[]
  }
  version: string
  database_bytes: number
  uptime_seconds: number
}

export type InstanceInviteStatus = 'active' | 'expired' | 'used_up' | 'revoked'

export interface InstanceInvite {
  code: string
  url: string
  note: string | null
  community: { id: string; name: string } | null
  max_uses: number | null
  uses: number
  expires_at: string | null
  revoked_at: string | null
  created_at: string
  created_by: UserPublic | null
  status: InstanceInviteStatus
  used_by: { user: UserPublic; used_at: string }[]
}

export interface AdminUser {
  id: string
  username: string
  display_name: string | null
  avatar_url: string | null
  created_at: string
  /** Tag der letzten Aktivität (`YYYY-MM-DD`). */
  last_active_on: string | null
  community_count: number
  is_instance_admin: boolean
  disabled_at: string | null
}

export interface AdminUserDetail extends AdminUser {
  communities: { id: string; name: string; is_owner: boolean; joined_at: string }[]
  session_count: number
  message_count: number
  storage_bytes: number
  registered_with: { code: string; note: string | null } | null
}

export type UserFilter = 'all' | 'admins' | 'disabled'

export interface AdminCommunity {
  id: string
  name: string
  owner: UserPublic
  member_count: number
  channel_count: number
  storage_bytes: number
  created_at: string
}

export interface AdminSettings {
  instance_name: string
  registration_mode: RegistrationMode
  max_upload_mb: number
  /** `null` = unbegrenzt. */
  user_quota_mb: number | null
  allow_community_creation: boolean
  announcement: string | null
  /** Bot-Konten erlaubt (ältere Server liefern das Feld nicht). */
  allow_bots?: boolean
  defaults: { instance_name: string; max_upload_mb: number; user_quota_mb: number | null }
  overridden: string[]
}

/** Fehlt ein Feld, bleibt es unverändert; `null` setzt auf den Standard (Umgebung) zurück. */
export interface AdminSettingsPatch {
  instance_name?: string | null
  registration_mode?: RegistrationMode
  max_upload_mb?: number | null
  /** `0` = unbegrenzt. */
  user_quota_mb?: number | null
  allow_community_creation?: boolean | null
  announcement?: string | null
  allow_bots?: boolean | null
}

export type OwnedCommunities = 'transfer' | 'delete'

function listQuery(params: Record<string, string | number | undefined>): string {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== '') q.set(k, String(v))
  const s = q.toString()
  return s ? `?${s}` : ''
}

export const adminApi = {
  overview: () => api<AdminOverview>('/admin/overview'),

  invites: () => api<InstanceInvite[]>('/admin/invites'),
  createInvite: (body: {
    max_uses?: number
    expires_in_hours?: number
    note?: string
    community_id?: string
  }) => api<InstanceInvite>('/admin/invites', 'POST', body),
  revokeInvite: (code: string) => api<void>(`/admin/invites/${enc(code)}`, 'DELETE'),

  users: (p: { q?: string; page?: number; per_page?: number; filter?: UserFilter }) =>
    api<Page<AdminUser>>(`/admin/users${listQuery({ ...p, filter: p.filter === 'all' ? undefined : p.filter })}`),
  user: (id: string) => api<AdminUserDetail>(`/admin/users/${enc(id)}`),
  disableUser: (id: string) => api<void>(`/admin/users/${enc(id)}/disable`, 'POST'),
  enableUser: (id: string) => api<void>(`/admin/users/${enc(id)}/enable`, 'POST'),
  revokeUserSessions: (id: string) => api<void>(`/admin/users/${enc(id)}/sessions`, 'DELETE'),
  grantAdmin: (id: string) => api<void>(`/admin/users/${enc(id)}/admin`, 'PUT'),
  revokeAdmin: (id: string) => api<void>(`/admin/users/${enc(id)}/admin`, 'DELETE'),
  passwordResetLink: (id: string) =>
    api<{ url: string; expires_at: string }>(`/admin/users/${enc(id)}/password-reset`, 'POST'),
  deleteUser: (id: string, body: { delete_messages: boolean; owned_communities?: OwnedCommunities }) =>
    api<void>(`/admin/users/${enc(id)}`, 'DELETE', body),

  communities: (p: { q?: string; page?: number; per_page?: number }) =>
    api<Page<AdminCommunity>>(`/admin/communities${listQuery(p)}`),
  communityMembers: (id: string) => api<UserPublic[]>(`/admin/communities/${enc(id)}/members`),
  deleteCommunity: (id: string) => api<void>(`/admin/communities/${enc(id)}`, 'DELETE'),
  transferCommunity: (id: string, userId: string) =>
    api<Community>(`/admin/communities/${enc(id)}/transfer`, 'POST', { user_id: userId }),

  settings: () => api<AdminSettings>('/admin/settings'),
  updateSettings: (body: AdminSettingsPatch) => api<AdminSettings>('/admin/settings', 'PATCH', body),

  auditLog: (before?: string) => api<AuditEntry[]>(`/admin/audit-log${listQuery({ before, limit: 50 })}`),

  // Öffentlich: Reset-Link einlösen (Seite `/reset-password#token=…`)
  verifyResetToken: (token: string) =>
    api<{ username: string; expires_at: string }>('/auth/password-reset/verify', 'POST', { token }),
  completeReset: (token: string, newPassword: string) =>
    api<void>('/auth/password-reset/complete', 'POST', { token, new_password: newPassword }),
}

/**
 * Token aus `/reset-password#token=…` (ältere Links: `?token=…`; nur im Web; `null` = keine Reset-Seite).
 */
export function resetTokenFromLocation(): string | null {
  if (!/^\/reset-password\/?$/.test(location.pathname)) return null
  return linkToken(location.hash, location.search) ?? ''
}
