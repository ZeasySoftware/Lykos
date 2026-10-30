import { create } from 'zustand'
import { INLINE_IMAGE_TYPES, deleteAttachment, uploadAttachment } from './api'
import type { Attachment } from './api'
import { errorText } from './errors'

/** Höchstzahl an Anhängen pro Nachricht (wie der Server). */
export const MAX_ATTACHMENTS = 10

export interface PendingUpload {
  localId: string
  name: string
  size: number
  /** 0..1 */
  progress: number
  status: 'uploading' | 'done' | 'error'
  error?: string
  attachment?: Attachment
  /** Lokale Vorschau für Bilder (Object-URL der ausgewählten Datei). */
  previewUrl?: string
}

interface UploadState {
  /** Anhänge im Composer, je Kanal – bleiben beim Kanalwechsel erhalten. */
  byChannel: Record<string, PendingUpload[]>
  /** Hinweis, wenn Dateien abgewiesen wurden (Anzahl). */
  notice: Record<string, string | undefined>
  add: (channelId: string, files: File[], maxBytes: number | undefined) => void
  remove: (channelId: string, localId: string) => void
  /** Nach erfolgreichem Senden: Liste leeren, ohne die (jetzt verknüpften) Uploads zu löschen. */
  clearSent: (channelId: string) => void
  dismissNotice: (channelId: string) => void
}

const aborts = new Map<string, () => void>()
let counter = 0

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toLocaleString('de-DE', { maximumFractionDigits: v < 10 ? 1 : 0 })} ${units[i]}`
}

/** Zwischenablage liefert Screenshots als „image.png“ – sprechenderen Namen vergeben. */
function displayName(file: File): string {
  if (file.name && file.name !== 'image.png') return file.name
  const stamp = new Date().toISOString().slice(0, 19).replace('T', '_').replaceAll(':', '-')
  return `Screenshot_${stamp}.png`
}

export const useUploads = create<UploadState>()((set, get) => {
  function update(channelId: string, localId: string, patch: Partial<PendingUpload>) {
    set((s) => ({
      byChannel: {
        ...s.byChannel,
        [channelId]: (s.byChannel[channelId] ?? []).map((u) => (u.localId === localId ? { ...u, ...patch } : u)),
      },
    }))
  }

  return {
    byChannel: {},
    notice: {},

    add(channelId, files, maxBytes) {
      const existing = get().byChannel[channelId] ?? []
      const free = MAX_ATTACHMENTS - existing.length
      const accepted = files.slice(0, Math.max(0, free))
      const notices: string[] = []
      if (files.length > accepted.length) notices.push(`Höchstens ${MAX_ATTACHMENTS} Anhänge pro Nachricht.`)

      const added: PendingUpload[] = []
      for (const file of accepted) {
        const localId = `u${++counter}`
        const name = displayName(file)
        if (maxBytes !== undefined && file.size > maxBytes) {
          notices.push(`„${name}“ ist zu groß (max. ${formatBytes(maxBytes)}).`)
          continue
        }
        const named = name === file.name ? file : new File([file], name, { type: file.type })
        const isImage = INLINE_IMAGE_TYPES.includes(file.type)
        const pending: PendingUpload = {
          localId,
          name,
          size: file.size,
          progress: 0,
          status: 'uploading',
          previewUrl: isImage ? URL.createObjectURL(file) : undefined,
        }
        added.push(pending)

        const handle = uploadAttachment(channelId, named, (progress) => update(channelId, localId, { progress }))
        aborts.set(localId, handle.abort)
        handle.promise
          .then((attachment) => {
            // Inzwischen entfernt? Dann den Upload serverseitig gleich wieder verwerfen.
            if (!get().byChannel[channelId]?.some((u) => u.localId === localId)) {
              void deleteAttachment(attachment.id).catch(() => {})
              return
            }
            update(channelId, localId, { status: 'done', progress: 1, attachment })
          })
          .catch((e) => update(channelId, localId, { status: 'error', error: errorText(e) }))
          .finally(() => aborts.delete(localId))
      }

      set((s) => ({
        byChannel: { ...s.byChannel, [channelId]: [...(s.byChannel[channelId] ?? []), ...added] },
        notice: { ...s.notice, [channelId]: notices.length ? notices.join(' ') : undefined },
      }))
    },

    remove(channelId, localId) {
      const upload = get().byChannel[channelId]?.find((u) => u.localId === localId)
      if (!upload) return
      aborts.get(localId)?.()
      if (upload.previewUrl) URL.revokeObjectURL(upload.previewUrl)
      if (upload.attachment) void deleteAttachment(upload.attachment.id).catch(() => {})
      set((s) => ({
        byChannel: { ...s.byChannel, [channelId]: (s.byChannel[channelId] ?? []).filter((u) => u.localId !== localId) },
      }))
    },

    clearSent(channelId) {
      for (const u of get().byChannel[channelId] ?? []) {
        if (u.previewUrl) URL.revokeObjectURL(u.previewUrl)
      }
      set((s) => {
        const byChannel = { ...s.byChannel }
        delete byChannel[channelId]
        return { byChannel }
      })
    },

    dismissNotice(channelId) {
      set((s) => ({ notice: { ...s.notice, [channelId]: undefined } }))
    },
  }
})
