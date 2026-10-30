import { useEffect, useState } from 'react'
import { attachmentUrl, attachmentsNeedToken, fetchAttachment } from './api'

/** Kantenlänge, auf die Profilbilder vor dem Upload verkleinert werden. */
export const AVATAR_SIZE = 256
/** Größere Dateien lehnt der Client vor dem Verarbeiten ab (Server-Limit: 8 MB). */
export const AVATAR_MAX_INPUT_BYTES = 20 * 1024 * 1024

/** Deterministische Hintergrundfarbe aus der Nutzer-ID (gleiche ID → gleiche Farbe auf allen Geräten). */
export function avatarColor(id: string): string {
  let hash = 0
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) | 0
  const hue = Math.abs(hash) % 360
  return `hsl(${hue} 42% 42%)`
}

/** Bis zu zwei Initialen aus dem Anzeigenamen. */
export function initials(name: string): string {
  const parts = name.trim().split(/[\s._-]+/).filter(Boolean)
  const letters = parts.length > 1 ? [parts[0], parts[1]] : [parts[0] ?? '?']
  return letters
    .map((p) => Array.from(p)[0] ?? '')
    .join('')
    .toUpperCase()
}

// ---------------------------------------------------------------------------
// Laden mit Token (Desktop) + Cache
// ---------------------------------------------------------------------------

/**
 * Object-URLs geladener Avatare je Pfad. Avatar-URLs ändern sich bei jedem neuen Bild, daher darf
 * der Cache für die Laufzeit der App halten (Anzahl begrenzt durch die Nutzer, die man sieht).
 */
const cache = new Map<string, string | null>()
const pending = new Map<string, Promise<string | null>>()

function loadAvatar(url: string): Promise<string | null> {
  let p = pending.get(url)
  if (!p) {
    p = fetchAttachment({ url })
      .then((blob) => URL.createObjectURL(blob))
      .catch(() => null)
      .then((src) => {
        cache.set(url, src)
        pending.delete(url)
        return src
      })
    pending.set(url, p)
  }
  return p
}

/**
 * Nutzbare `src` für ein Profilbild. Web: direkt (Cookie, Browser-Cache). Desktop-Bundle: einmal per
 * fetch mit Token laden und als Object-URL wiederverwenden. `null`, solange nicht geladen/fehlerhaft.
 */
export function useAvatarSrc(url: string | null | undefined): string | null {
  const direct = !url ? null : attachmentsNeedToken() ? undefined : attachmentUrl({ url })
  const [loaded, setLoaded] = useState<{ url: string; src: string | null } | null>(null)

  useEffect(() => {
    if (!url || direct !== undefined || cache.has(url)) return
    let cancelled = false
    void loadAvatar(url).then((src) => {
      if (!cancelled) setLoaded({ url, src })
    })
    return () => {
      cancelled = true
    }
  }, [url, direct])

  if (!url) return null
  if (direct !== undefined) return direct
  if (cache.has(url)) return cache.get(url) ?? null
  return loaded?.url === url ? loaded.src : null
}

// ---------------------------------------------------------------------------
// Zuschneiden vor dem Upload
// ---------------------------------------------------------------------------

/**
 * Schneidet ein Bild mittig quadratisch zu, verkleinert es auf höchstens `AVATAR_SIZE` px und kodiert es
 * neu (WebP, sonst PNG). Neu kodieren entfernt Metadaten wie EXIF/GPS. Animierte GIFs werden zum
 * Standbild (erstes Bild). Wirft bei nicht lesbaren Dateien.
 */
export async function prepareAvatar(file: File): Promise<{ blob: Blob; filename: string }> {
  const bitmap = await createImageBitmap(file)
  try {
    const side = Math.min(bitmap.width, bitmap.height)
    const size = Math.max(1, Math.min(AVATAR_SIZE, side))
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas nicht verfügbar')
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, size, size)

    const toBlob = (type: string) =>
      new Promise<Blob | null>((resolve) => canvas.toBlob((b) => resolve(b), type, 0.9))
    const webp = await toBlob('image/webp')
    if (webp && webp.type === 'image/webp') return { blob: webp, filename: 'avatar.webp' }
    const png = await toBlob('image/png')
    if (!png) throw new Error('Bild konnte nicht kodiert werden')
    return { blob: png, filename: 'avatar.png' }
  } finally {
    bitmap.close()
  }
}
