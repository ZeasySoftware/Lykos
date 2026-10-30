/**
 * Custom-Emotes im Text (`<:name:id>`, animiert `<a:name:id>`) und als Reaktion (`name:id`), plus Hilfen für
 * Emoji-Erkennung und die `:shortcode:`-Eingabe. Reine Logik ohne React/Store – getestet in `customEmoji.test.ts`.
 *
 * Beispiele:
 *   parseCustomToken('<a:tanz:0192f3a0-1111-7000-8000-000000000001>', 0)
 *     → { name: 'tanz', id: '0192…', animated: true, length: 46 }
 *   emojiOnlyCount('🐺 <:wolf:…> 👍🏽') → 3   (für große Darstellung)
 *   emojiQuery('Hallo :wol', 10) → { start: 6, query: 'wol' }
 */

/** Name eines Custom-Emotes: 2–32 Zeichen `[A-Za-z0-9_]` (wie auf dem Server). */
export const EMOJI_NAME_RE = /^[A-Za-z0-9_]{2,32}$/

const UUID = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
const TOKEN_AT = new RegExp(`^<(a?):([A-Za-z0-9_]{2,32}):(${UUID})>`)
const TOKEN_ALL = new RegExp(`<a?:[A-Za-z0-9_]{2,32}:${UUID}>`, 'g')
const REACTION = new RegExp(`^([A-Za-z0-9_]{2,32}):(${UUID})$`)

/** Höchstzahl an Emojis, bis zu der eine reine Emoji-Nachricht groß dargestellt wird (wie Discord). */
export const JUMBO_MAX = 27

export interface CustomEmojiRef {
  id: string
  name: string
  animated: boolean
}

/** Token ab Position `i` lesen (`text[i] === '<'`); `null`, wenn dort keins steht. */
export function parseCustomToken(text: string, i: number): (CustomEmojiRef & { length: number }) | null {
  if (text[i] !== '<') return null
  const m = TOKEN_AT.exec(text.slice(i, i + 80))
  if (!m) return null
  return { animated: m[1] === 'a', name: m[2], id: m[3].toLowerCase(), length: m[0].length }
}

/** Token für den Nachrichtentext. */
export function formatCustomToken(e: CustomEmojiRef): string {
  return `<${e.animated ? 'a' : ''}:${e.name}:${e.id}>`
}

/** Reaktionsschlüssel für die API (`name:id`). */
export function customReaction(e: Pick<CustomEmojiRef, 'name' | 'id'>): string {
  return `${e.name}:${e.id}`
}

/** `name:id` einer Reaktion zerlegen; `null` bei Unicode-Emojis. */
export function parseCustomReaction(emoji: string): { name: string; id: string } | null {
  const m = REACTION.exec(emoji)
  return m ? { name: m[1], id: m[2].toLowerCase() } : null
}

/** Vergleichsschlüssel einer Reaktion: Emote-ID bei Custom-Emotes (Name kann sich ändern), sonst das Emoji. */
export function reactionKey(emoji: string): string {
  return parseCustomReaction(emoji)?.id ?? emoji
}

/** Bild-Pfad eines Emotes (der Server ignoriert die Endung und liefert den echten Typ). */
export function customEmojiPath(id: string, animated: boolean): string {
  return `/api/emojis/${id}.${animated ? 'gif' : 'png'}`
}

/** Ein Graphem, das als Emoji gilt (Piktogramm, Flagge, Tastenkappe). */
const EMOJI_GRAPHEME = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[#*0-9]️?⃣)/u

let segmenter: Intl.Segmenter | null = null

/**
 * Besteht der Text nur aus Emojis (Unicode oder Custom) und Leerraum? Liefert deren Anzahl, sonst 0.
 * Mehr als `JUMBO_MAX` → 0 (dann nicht groß darstellen).
 */
export function emojiOnlyCount(text: string): number {
  if (!text.trim()) return 0
  let count = 0
  const rest = text.replace(TOKEN_ALL, () => {
    count++
    return ' '
  })
  segmenter ??= new Intl.Segmenter(undefined, { granularity: 'grapheme' })
  for (const { segment } of segmenter.segment(rest)) {
    if (/^\s+$/u.test(segment)) continue
    if (!EMOJI_GRAPHEME.test(segment)) return 0
    // Text-Varianten wie © oder ™ ohne Emoji-Selektor sind eher Zeichen als Emojis.
    if (/^[©®™]$/u.test(segment)) return 0
    count++
    if (count > JUMBO_MAX) return 0
  }
  return count
}

/** Offene `:shortcode`-Eingabe direkt vor dem Cursor (ab 2 Zeichen), z. B. `Hi :wo|` → `{ start: 3, query: 'wo' }`. */
export function emojiQuery(text: string, caret: number): { start: number; query: string } | null {
  // Doppelpunkt nicht direkt nach Wortzeichen (Uhrzeiten, URLs `http:`) und nicht in `<:name:…>`.
  const m = /(^|[^A-Za-z0-9_:<])(:([A-Za-z0-9_+-]{2,32}))$/.exec(text.slice(0, caret))
  if (!m) return null
  return { start: caret - m[2].length, query: m[3] }
}

/**
 * Wendet `fn` nur auf Text außerhalb von Code an (```Block``` und `inline` innerhalb einer Zeile, jeweils nur
 * mit schließendem Begrenzer – wie der Markdown-Parser).
 */
export function mapOutsideCode(text: string, fn: (plain: string) => string): string {
  let out = ''
  let i = 0
  let plain = ''
  const flush = () => {
    out += plain ? fn(plain) : ''
    plain = ''
  }
  while (i < text.length) {
    if (text.startsWith('```', i)) {
      const end = text.indexOf('```', i + 3)
      if (end !== -1) {
        flush()
        out += text.slice(i, end + 3)
        i = end + 3
        continue
      }
    }
    if (text[i] === '`') {
      const end = text.indexOf('`', i + 1)
      if (end > i + 1 && !text.slice(i + 1, end).includes('\n')) {
        flush()
        out += text.slice(i, end + 1)
        i = end + 1
        continue
      }
    }
    plain += text[i]
    i++
  }
  flush()
  return out
}

/**
 * Ersetzt vollständige `:code:` außerhalb von Code durch `resolve(code)` (z. B. Custom-Emote-Token oder
 * Unicode-Emoji); `null` lässt den Text stehen. Teile von Tokens (`<:name:id>`) und Uhrzeiten bleiben unberührt.
 */
export function replaceShortcodes(text: string, resolve: (code: string) => string | null): string {
  return mapOutsideCode(text, (plain) =>
    plain.replace(/(?<![A-Za-z0-9_<:]):([A-Za-z0-9_+-]{2,32}):(?![A-Za-z0-9_])/g, (all, code: string) => resolve(code) ?? all),
  )
}

/** Tokens außerhalb von Code durch `:name:` ersetzen, wenn `shortName(token)` einen Namen liefert (zum Bearbeiten). */
export function shortenCustomTokens(text: string, shortName: (e: CustomEmojiRef) => string | null): string {
  return mapOutsideCode(text, (plain) =>
    plain.replace(TOKEN_ALL, (token) => {
      const e = parseCustomToken(token, 0)
      const name = e && shortName(e)
      return name ? `:${name}:` : token
    }),
  )
}

/** Gerade abgeschlossenes `:shortcode:` direkt vor dem Cursor (für das automatische Ersetzen). */
export function completedShortcode(text: string, caret: number): { start: number; code: string } | null {
  const m = /(^|[^A-Za-z0-9_:<])(:([A-Za-z0-9_+-]{2,32}):)$/.exec(text.slice(0, caret))
  if (!m) return null
  return { start: caret - m[2].length, code: m[3] }
}
