import type { CSSProperties } from 'react'
import type { Community } from './api'

/**
 * Gestaltung (Paket J): Bildaufbereitung für Community-Icon/-Banner und Profilbanner sowie Farbhilfen.
 *
 * Bilder werden vor dem Upload mittig auf das Zielformat zugeschnitten, verkleinert und neu kodiert
 * (Canvas → WebP, sonst PNG) – das entfernt Metadaten wie EXIF/GPS. Nur animierte Community-Icons bis 1 MB
 * gehen unverändert hoch (GIF trägt keine EXIF-Daten; die Animation bliebe sonst nicht erhalten).
 */

/** Zielformate (Breite × Höhe in px). */
export const IMAGE_FORMATS = {
  communityIcon: { width: 256, height: 256 },
  communityBanner: { width: 960, height: 540 },
  profileBanner: { width: 600, height: 240 },
} as const

export type ImageFormat = (typeof IMAGE_FORMATS)[keyof typeof IMAGE_FORMATS]

/** Größere Eingabedateien lehnt der Client vor dem Verarbeiten ab. */
export const IMAGE_MAX_INPUT_BYTES = 20 * 1024 * 1024
/** Animierte GIF-Icons bis zu dieser Größe werden unverändert hochgeladen. */
export const GIF_PASSTHROUGH_BYTES = 1024 * 1024

async function isGif(file: Blob): Promise<boolean> {
  const head = new Uint8Array(await file.slice(0, 6).arrayBuffer())
  return String.fromCharCode(...head).startsWith('GIF8')
}

/**
 * Schneidet ein Bild mittig auf das Seitenverhältnis von `format` zu, verkleinert es auf höchstens dessen
 * Größe (nie vergrößern) und kodiert es neu. `keepGif`: kleine GIFs unverändert lassen (Animation).
 * Wirft bei nicht lesbaren Dateien.
 */
export async function prepareImage(
  file: File,
  format: ImageFormat,
  name: string,
  keepGif = false,
): Promise<{ blob: Blob; filename: string }> {
  if (keepGif && file.size <= GIF_PASSTHROUGH_BYTES && (await isGif(file))) {
    return { blob: file, filename: `${name}.gif` }
  }
  const bitmap = await createImageBitmap(file)
  try {
    const aspect = format.width / format.height
    // Größter Ausschnitt mit Zielverhältnis, mittig.
    let sw = bitmap.width
    let sh = sw / aspect
    if (sh > bitmap.height) {
      sh = bitmap.height
      sw = sh * aspect
    }
    const scale = Math.min(1, format.width / sw)
    const width = Math.max(1, Math.round(sw * scale))
    const height = Math.max(1, Math.round(sh * scale))
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas nicht verfügbar')
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(bitmap, (bitmap.width - sw) / 2, (bitmap.height - sh) / 2, sw, sh, 0, 0, width, height)

    const toBlob = (type: string) =>
      new Promise<Blob | null>((resolve) => canvas.toBlob((b) => resolve(b), type, 0.9))
    const webp = await toBlob('image/webp')
    if (webp && webp.type === 'image/webp') return { blob: webp, filename: `${name}.webp` }
    const png = await toBlob('image/png')
    if (!png) throw new Error('Bild konnte nicht kodiert werden')
    return { blob: png, filename: `${name}.png` }
  } finally {
    bitmap.close()
  }
}

// ---------------------------------------------------------------------------
// Farben (0xRRGGBB als Zahl, wie Rollenfarben)
// ---------------------------------------------------------------------------

/** Vorschläge für Akzent- und Bannerfarben – gedeckt, damit sie zum dunklen Design passen. */
export const COLOR_SUGGESTIONS: number[] = [
  0x6d5bd0, // Lykos-Violett
  0x3b82c4, // Blau
  0x2f9e8f, // Petrol
  0x3fa060, // Grün
  0xc9a227, // Gold
  0xd9783b, // Orange
  0xc4413c, // Rot
  0xc0508f, // Magenta
  0x5f6b85, // Schiefer
]

export function colorToHex(color: number): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, '0')}`
}

export function hexToColor(hex: string): number | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim())
  return m ? parseInt(m[1], 16) : null
}

/** CSS-Farbe mit Deckkraft (0–1). */
export function colorAlpha(color: number, alpha: number): string {
  return `rgb(${(color >> 16) & 255} ${(color >> 8) & 255} ${color & 255} / ${alpha})`
}

/**
 * Hintergrund eines Banners ohne Bild: Bannerfarbe (Profil) bzw. dezenter Verlauf aus der Akzentfarbe;
 * ohne beides `undefined` (dann greift der Standard-Verlauf aus dem Stylesheet).
 */
export function bannerFallback(color: number | null | undefined, accent?: number | null): string | undefined {
  if (color != null) return colorToHex(color)
  if (accent != null) {
    return `radial-gradient(120% 140% at 0% 0%, ${colorAlpha(accent, 0.55)}, transparent 62%), linear-gradient(120deg, ${colorAlpha(accent, 0.28)}, var(--bg-content))`
  }
  return undefined
}

/** Bis zu zwei Initialen eines Community-Namens. */
export function communityInitials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean)
  if (words.length === 0) return '?'
  if (words.length === 1) return Array.from(words[0]).slice(0, 2).join('').toUpperCase()
  return ((Array.from(words[0])[0] ?? '') + (Array.from(words[1])[0] ?? '')).toUpperCase()
}

/** Inline-Style für Elemente, die die Akzentfarbe als `--custom-accent` nutzen. */
export function accentStyle(accent: number | null | undefined): CSSProperties | undefined {
  return accent != null ? ({ '--custom-accent': colorToHex(accent) } as CSSProperties) : undefined
}

/** Zusatzklasse für `.sidebar-header`: hoch mit Bannerbild, sonst ggf. dezent eingefärbt. */
export function sidebarHeaderClass(community: Community | undefined): string {
  if (community?.banner_url) return ' has-banner'
  if (community?.accent_color != null) return ' has-accent'
  return ''
}
