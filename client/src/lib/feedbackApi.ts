/**
 * Fehler- und Vorschlagsmeldungen (Paket K): Typen, Endpunkte und Screenshot-Aufbereitung.
 * Siehe docs/api.md → „Feedback“.
 */
import { ApiError, api, attachmentsNeedToken, fetchAttachment, getServerUrl, getToken } from './api'
import type { UserPublic } from './api'
import type { TechInfo } from './diagnostics'

export type FeedbackKind = 'bug' | 'suggestion' | 'other'
export type FeedbackStatus = 'neu' | 'in_arbeit' | 'erledigt' | 'abgelehnt' | 'duplikat'

export const KIND_LABELS: Record<FeedbackKind, string> = {
  bug: 'Fehler',
  suggestion: 'Vorschlag',
  other: 'Sonstiges',
}

export const STATUS_LABELS: Record<FeedbackStatus, string> = {
  neu: 'Neu',
  in_arbeit: 'In Arbeit',
  erledigt: 'Erledigt',
  abgelehnt: 'Abgelehnt',
  duplikat: 'Duplikat',
}

export const STATUSES = Object.keys(STATUS_LABELS) as FeedbackStatus[]
export const KINDS = Object.keys(KIND_LABELS) as FeedbackKind[]

export const TITLE_MIN = 3
export const TITLE_MAX = 150
export const DESCRIPTION_MAX = 10_000
export const REPLY_MAX = 4000

export interface FeedbackReply {
  id: string
  author: UserPublic | null
  body: string
  /** Interne Notiz (nur Admins sehen sie). */
  internal: boolean
  created_at: string
}

export interface Feedback {
  id: string
  /** Meldungsnummer. */
  number: number
  kind: FeedbackKind
  status: FeedbackStatus
  title: string
  description: string
  vote_count: number
  voted: boolean
  mine: boolean
  replies: FeedbackReply[]
  created_at: string
  updated_at: string
  /** Nur Melder und Admins. */
  tech_info?: TechInfo | null
  screenshot_url?: string | null
  /** Nur Admins. */
  reporter?: UserPublic | null
  issue_url?: string | null
}

export interface VoteState {
  vote_count: number
  voted: boolean
}

const enc = encodeURIComponent

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/** Datum und Uhrzeit für Meldungen und Antworten. */
export function formatDate(iso: string): string {
  return dateFmt.format(new Date(iso))
}

export const feedbackEndpoints = {
  create: (body: { kind: FeedbackKind; title: string; description: string; tech_info: TechInfo | null }) =>
    api<Feedback>('/feedback', 'POST', body),
  mine: () => api<Feedback[]>('/feedback/mine?limit=100'),
  suggestions: (sort: 'votes' | 'new', offset = 0, limit = 50) =>
    api<Feedback[]>(`/feedback/suggestions?sort=${sort}&offset=${offset}&limit=${limit}`),
  get: (id: string) => api<Feedback>(`/feedback/${enc(id)}`),
  withdraw: (id: string) => api<void>(`/feedback/${enc(id)}`, 'DELETE'),
  vote: (id: string) => api<VoteState>(`/feedback/${enc(id)}/vote`, 'PUT'),
  unvote: (id: string) => api<VoteState>(`/feedback/${enc(id)}/vote`, 'DELETE'),

  adminSettings: () => api<{ gitea: boolean }>('/admin/feedback/settings'),
  adminList: (filter: { kind?: FeedbackKind; status?: FeedbackStatus; before?: string; limit?: number }) => {
    const q = new URLSearchParams({ limit: String(filter.limit ?? 50) })
    if (filter.kind) q.set('kind', filter.kind)
    if (filter.status) q.set('status', filter.status)
    if (filter.before) q.set('before', filter.before)
    return api<Feedback[]>(`/admin/feedback?${q}`)
  },
  adminGet: (id: string) => api<Feedback>(`/admin/feedback/${enc(id)}`),
  adminSetStatus: (id: string, status: FeedbackStatus) =>
    api<Feedback>(`/admin/feedback/${enc(id)}`, 'PATCH', { status }),
  adminReply: (id: string, body: string, internal: boolean) =>
    api<Feedback>(`/admin/feedback/${enc(id)}/replies`, 'POST', { body, internal }),
  adminDelete: (id: string) => api<void>(`/admin/feedback/${enc(id)}`, 'DELETE'),
  adminCreateIssue: (id: string) => api<Feedback>(`/admin/feedback/${enc(id)}/issue`, 'POST'),
}

/** Screenshot zu einer gerade abgesendeten Meldung hochladen (bereits aufbereitet, siehe `prepareScreenshot`). */
export async function uploadScreenshot(id: string, blob: Blob, filename: string): Promise<Feedback> {
  const form = new FormData()
  form.append('file', blob, filename)
  const headers: Record<string, string> = {}
  const token = getToken()
  if (token) headers.Authorization = `Bearer ${token}`
  let res: Response
  try {
    res = await fetch(`${getServerUrl()}/api/feedback/${enc(id)}/screenshot`, {
      method: 'POST',
      headers,
      body: form,
      credentials: attachmentsNeedToken() ? 'omit' : 'same-origin',
    })
  } catch {
    throw new ApiError(0, 'network')
  }
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new ApiError(res.status, data.error ?? (res.status === 413 ? 'file_too_large' : 'unknown'))
  }
  return (await res.json()) as Feedback
}

/** Screenshot als Blob laden (mit Token, falls nötig). */
export function fetchScreenshot(url: string, signal?: AbortSignal): Promise<Blob> {
  return fetchAttachment({ url }, signal)
}

// ---------------------------------------------------------------------------
// Screenshot vorbereiten
// ---------------------------------------------------------------------------

/** Größte Kantenlänge nach dem Verkleinern. */
export const SCREENSHOT_MAX_SIDE = 2560
/** Serverlimit für Screenshots. */
export const SCREENSHOT_MAX_BYTES = 5 * 1024 * 1024
/** Größte akzeptierte Eingabedatei. */
export const SCREENSHOT_MAX_INPUT_BYTES = 25 * 1024 * 1024

/**
 * Verkleinert ein Bild auf höchstens `SCREENSHOT_MAX_SIDE` px und kodiert es neu (WebP, sonst PNG/JPEG) –
 * das entfernt Metadaten wie EXIF/GPS. Wirft bei nicht lesbaren Dateien oder wenn das Ergebnis zu groß bleibt.
 */
export async function prepareScreenshot(file: Blob): Promise<{ blob: Blob; filename: string }> {
  if (file.size > SCREENSHOT_MAX_INPUT_BYTES) throw new ApiError(413, 'file_too_large')
  const bitmap = await createImageBitmap(file)
  try {
    const scale = Math.min(1, SCREENSHOT_MAX_SIDE / Math.max(bitmap.width, bitmap.height))
    const width = Math.max(1, Math.round(bitmap.width * scale))
    const height = Math.max(1, Math.round(bitmap.height * scale))
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas nicht verfügbar')
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(bitmap, 0, 0, width, height)

    const toBlob = (type: string, quality?: number) =>
      new Promise<Blob | null>((resolve) => canvas.toBlob((b) => resolve(b), type, quality))
    const candidates: [string, number | undefined, string][] = [
      ['image/webp', 0.9, 'screenshot.webp'],
      ['image/png', undefined, 'screenshot.png'],
      ['image/jpeg', 0.85, 'screenshot.jpg'],
      ['image/jpeg', 0.6, 'screenshot.jpg'],
    ]
    for (const [type, quality, filename] of candidates) {
      const blob = await toBlob(type, quality)
      if (blob && blob.type === type && blob.size <= SCREENSHOT_MAX_BYTES) return { blob, filename }
    }
    throw new ApiError(413, 'file_too_large')
  } finally {
    bitmap.close()
  }
}
