/** Vorbereitung von Custom-Emote-Uploads im Client (Server-Limits siehe docs/api.md → Custom-Emotes). */

export const EMOJI_MAX_BYTES = 256 * 1024
export const EMOJI_MAX_SIDE = 128
export const EMOJI_LIMIT = 100
/** Größere Dateien lehnt der Client vor dem Verarbeiten ab. */
const MAX_INPUT_BYTES = 20 * 1024 * 1024

/** Namensvorschlag aus dem Dateinamen: `[A-Za-z0-9_]`, 2–32 Zeichen. */
export function suggestEmojiName(filename: string): string {
  const base = filename.replace(/\.[^.]*$/, '')
  const name = base
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .replace(/ß/g, 'ss')
    .replace(/[^A-Za-z0-9_]+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_|_$/g, '')
    .slice(0, 32)
  return name.length >= 2 ? name : `emoji_${name}`.slice(0, 32).replace(/_$/, '') || 'emoji'
}

/** Name eindeutig machen (`wolf` → `wolf_2`, …), ohne Groß/Klein-Unterschied wie der Server. */
export function uniqueEmojiName(name: string, taken: Iterable<string>): string {
  const set = new Set([...taken].map((n) => n.toLowerCase()))
  if (!set.has(name.toLowerCase())) return name
  for (let i = 2; i < 1000; i++) {
    const suffix = `_${i}`
    const candidate = name.slice(0, 32 - suffix.length) + suffix
    if (!set.has(candidate.toLowerCase())) return candidate
  }
  return name
}

async function sniff(file: Blob): Promise<Uint8Array> {
  return new Uint8Array(await file.slice(0, 16).arrayBuffer())
}

function isGif(head: Uint8Array): boolean {
  return head[0] === 0x47 && head[1] === 0x49 && head[2] === 0x46 && head[3] === 0x38
}

/**
 * Bereitet eine Bilddatei als Emote vor. Statische Bilder (PNG, WebP, JPEG, …), die zu groß sind oder kein
 * PNG/WebP sind, werden per Canvas auf höchstens 128 px verkleinert und als WebP (sonst PNG) neu kodiert.
 * Animierte GIFs werden **nie** neu kodiert (das zerstört die Animation) – sie müssen die Limits schon einhalten.
 * Wirft `Error` mit einer deutschen Meldung.
 */
export async function prepareEmoji(file: File): Promise<{ blob: Blob; filename: string }> {
  if (file.size > MAX_INPUT_BYTES) throw new Error('Die Datei ist zu groß.')
  const head = await sniff(file)
  if (isGif(head)) {
    const width = head[6] | (head[7] << 8)
    const height = head[8] | (head[9] << 8)
    if (width > EMOJI_MAX_SIDE || height > EMOJI_MAX_SIDE || file.size > EMOJI_MAX_BYTES) {
      throw new Error(
        `Animierte GIFs bitte vorher auf höchstens ${EMOJI_MAX_SIDE} × ${EMOJI_MAX_SIDE} px und 256 KB bringen (dieses: ${width} × ${height} px, ${Math.ceil(file.size / 1024)} KB).`,
      )
    }
    return { blob: file, filename: file.name || 'emoji.gif' }
  }

  let bitmap: ImageBitmap
  try {
    bitmap = await createImageBitmap(file)
  } catch {
    throw new Error('Bitte ein Bild im Format PNG, GIF, WebP oder JPEG wählen.')
  }
  try {
    const keep =
      (file.type === 'image/png' || file.type === 'image/webp') &&
      file.size <= EMOJI_MAX_BYTES &&
      bitmap.width <= EMOJI_MAX_SIDE &&
      bitmap.height <= EMOJI_MAX_SIDE
    if (keep) return { blob: file, filename: file.name }

    const scale = Math.min(1, EMOJI_MAX_SIDE / Math.max(bitmap.width, bitmap.height))
    const width = Math.max(1, Math.round(bitmap.width * scale))
    const height = Math.max(1, Math.round(bitmap.height * scale))
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas nicht verfügbar.')
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(bitmap, 0, 0, width, height)
    const toBlob = (type: string) => new Promise<Blob | null>((resolve) => canvas.toBlob((b) => resolve(b), type, 0.92))
    const webp = await toBlob('image/webp')
    const blob = webp && webp.type === 'image/webp' ? webp : await toBlob('image/png')
    if (!blob) throw new Error('Bild konnte nicht kodiert werden.')
    if (blob.size > EMOJI_MAX_BYTES) throw new Error('Das Bild ist auch verkleinert noch größer als 256 KB.')
    return { blob, filename: blob.type === 'image/webp' ? 'emoji.webp' : 'emoji.png' }
  } finally {
    bitmap.close()
  }
}
