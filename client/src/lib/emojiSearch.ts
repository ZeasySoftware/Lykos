/**
 * Unicode-Emoji-Daten aufbereiten und durchsuchen (deutsche + englische Schlüsselwörter, Shortcodes), plus
 * Vorschläge für die `:shortcode:`-Autovervollständigung inkl. Custom-Emotes. Reine Logik – getestet in
 * `emojiSearch.test.ts`; die Daten selbst lädt `emojiData.ts` per dynamischem Import.
 *
 * Beispiele:
 *   searchEmojis(list, 'daumen')[0].emoji → '👍'
 *   searchEmojis(list, 'thumbsup')[0].emoji → '👍'
 *   applySkin('👍', 3) → '👍🏽'
 */

/** Zeile aus `src/data/emoji-data.json` (Format siehe `scripts/gen-emoji-data.mjs`). */
export type RawEmojiRow = [string, string, string, string, number, (1 | string[])?]

/** Kategorien in Anzeigereihenfolge (Index = `group` in den Daten). */
export const EMOJI_GROUPS = [
  'Smileys & Personen',
  'Tiere & Natur',
  'Essen & Trinken',
  'Aktivitäten',
  'Reisen & Orte',
  'Objekte',
  'Symbole',
  'Flaggen',
] as const

export interface UnicodeEmoji {
  emoji: string
  /** Shortcodes ohne Doppelpunkte, der erste ist der bevorzugte (z. B. `thumbsup`). */
  shortcodes: string[]
  /** Deutscher Name (CLDR), z. B. „Daumen hoch“. */
  name: string
  group: number
  /** Fünf Hautton-Varianten (hell → dunkel), sonst `null`. */
  skins: string[] | null
  /** Gefaltete Shortcodes (vollständig, gleiche Reihenfolge wie `shortcodes`). */
  folded: string[]
  /** Gefaltete Shortcodes, zusätzlich in Teilen (`thumbs_up` → `thumbs`, `up`). */
  codes: string[]
  nameWords: string[]
  keywords: string[]
}

/** Custom-Emote, soweit Suche und Vorschläge es brauchen. */
export interface CustomEmojiLite {
  id: string
  name: string
  animated: boolean
  community_id: string
}

const TONES = ['\u{1F3FB}', '\u{1F3FC}', '\u{1F3FD}', '\u{1F3FE}', '\u{1F3FF}']

/** Kleinbuchstaben ohne diakritische Zeichen (`Lächeln` → `lacheln`, `ß` → `ss`). */
export function fold(s: string): string {
  return s.toLowerCase().normalize('NFD').replace(/\p{M}/gu, '').replace(/ß/g, 'ss')
}

/** Standardregel für Hauttöne: Modifier nach dem ersten Codepunkt, ein folgender Varianten-Selektor entfällt. */
function toneRule(emoji: string, tone: number): string {
  const cps = Array.from(emoji)
  const rest = cps.slice(1)
  if (rest[0] === '️') rest.shift()
  return cps[0] + TONES[tone] + rest.join('')
}

/** Emoji mit Hautton `tone` (0 = Standard, 1–5 = hell → dunkel), falls es Varianten hat. */
export function applySkin(e: Pick<UnicodeEmoji, 'emoji' | 'skins'>, tone: number): string {
  if (!e.skins || tone < 1 || tone > 5) return e.emoji
  return e.skins[tone - 1] ?? e.emoji
}

function words(s: string): string[] {
  return fold(s)
    .split(/[\s_:,.()’'-]+/)
    .filter(Boolean)
}

/** Rohdaten in durchsuchbare Einträge umwandeln (einmal pro Sitzung). */
export function prepareEmojis(rows: RawEmojiRow[]): UnicodeEmoji[] {
  return rows.map(([emoji, codes, name, keywords, group, skins]) => {
    const shortcodes = codes.split(' ').filter(Boolean)
    const folded = shortcodes.map(fold)
    return {
      emoji,
      shortcodes,
      name,
      group,
      skins: skins === 1 ? TONES.map((_, i) => toneRule(emoji, i)) : (skins ?? null),
      folded,
      codes: [...new Set([...folded, ...folded.flatMap((c) => c.split(/[_-]/))])],
      nameWords: words(name),
      keywords: keywords.split(' ').filter(Boolean).map(fold),
    }
  })
}

/** Rang eines Suchbegriffs für ein Emoji (kleiner = besser), `-1` = kein Treffer. */
function termRank(e: UnicodeEmoji, term: string): number {
  if (e.folded.includes(term)) return 0
  // Ganze Wörter vor Wortanfängen: „herz“ soll Herzen finden, nicht zuerst „Herzegowina“.
  if (e.codes.includes(term) || e.nameWords.includes(term)) return 1
  if (e.codes.some((c) => c.startsWith(term))) return 2
  if (e.nameWords.some((w) => w.startsWith(term))) return 3
  if (e.keywords.includes(term)) return 3
  if (e.keywords.some((w) => w.startsWith(term))) return 4
  if (term.length >= 3 && (e.codes.some((c) => c.includes(term)) || fold(e.name).includes(term))) return 6
  return -1
}

/**
 * Sucht Emojis: Jeder Begriff muss als Wortanfang in Shortcode, Name oder Schlüsselwörtern vorkommen
 * (deutsch oder englisch, Umlaute egal). Sortiert nach Trefferqualität, dann nach Standardreihenfolge.
 */
export function searchEmojis(list: UnicodeEmoji[], query: string, limit = 160): UnicodeEmoji[] {
  const terms = fold(query)
    .replace(/^:|:$/g, '')
    .split(/[\s_]+/)
    .filter(Boolean)
  if (terms.length === 0) return []
  const hits: { e: UnicodeEmoji; rank: number; i: number }[] = []
  list.forEach((e, i) => {
    let rank = 0
    for (const t of terms) {
      const r = termRank(e, t)
      if (r < 0) return
      rank += r
    }
    hits.push({ e, rank, i })
  })
  hits.sort((a, b) => a.rank - b.rank || a.i - b.i)
  return hits.slice(0, limit).map((h) => h.e)
}

/** Custom-Emotes nach Namen filtern (Präfix vor Teilstring). */
export function searchCustom<T extends CustomEmojiLite>(list: T[], query: string): T[] {
  const q = fold(query).replace(/^:|:$/g, '').replace(/\s+/g, '_')
  if (!q) return []
  const scored = list
    .map((e) => {
      const n = fold(e.name)
      return { e, rank: n === q ? 0 : n.startsWith(q) ? 1 : n.includes(q) ? 2 : -1 }
    })
    .filter((x) => x.rank >= 0)
  scored.sort((a, b) => a.rank - b.rank || a.e.name.localeCompare(b.e.name, 'de'))
  return scored.map((x) => x.e)
}

export type ShortcodeSuggestion<T extends CustomEmojiLite = CustomEmojiLite> =
  | { kind: 'unicode'; emoji: UnicodeEmoji; code: string }
  | { kind: 'custom'; emoji: T }

/**
 * Vorschläge für `:query`: zuerst Shortcodes, die mit der Eingabe beginnen (Custom-Emotes und Unicode gemischt,
 * exakte Treffer vorn), dann Teiltreffer und Treffer über Name/Schlüsselwörter. Höchstens `limit`.
 */
export function shortcodeSuggestions<T extends CustomEmojiLite>(
  unicode: UnicodeEmoji[] | null,
  custom: T[],
  query: string,
  limit = 8,
): ShortcodeSuggestion<T>[] {
  const q = fold(query)
  if (q.length < 2) return []
  const out: { s: ShortcodeSuggestion<T>; rank: number; order: number }[] = []
  custom.forEach((e, i) => {
    const n = fold(e.name)
    const rank = n === q ? 0 : n.startsWith(q) ? 1 : n.includes(q) ? 3 : -1
    if (rank >= 0) out.push({ s: { kind: 'custom', emoji: e }, rank, order: i })
  })
  unicode?.forEach((e, i) => {
    let idx = e.folded.findIndex((c) => c.startsWith(q))
    if (idx < 0) idx = e.folded.findIndex((c) => c.includes(q))
    const code = idx >= 0 ? e.shortcodes[idx] : undefined
    let rank = -1
    if (idx >= 0) rank = e.folded[idx] === q ? 0 : e.folded[idx].startsWith(q) ? 1 : 3
    else if (q.length >= 3 && (e.nameWords.some((w) => w.startsWith(q)) || e.keywords.some((w) => w.startsWith(q)))) rank = 4
    if (rank >= 0) out.push({ s: { kind: 'unicode', emoji: e, code: code ?? e.shortcodes[0] }, rank, order: custom.length + i })
  })
  out.sort((a, b) => a.rank - b.rank || a.order - b.order)
  return out.slice(0, limit).map((x) => x.s)
}

/** Exakter Treffer für ein vollständiges `:code:` (Custom-Emotes zuerst), sonst `null`. */
export function findShortcode<T extends CustomEmojiLite>(
  unicode: UnicodeEmoji[] | null,
  custom: T[],
  code: string,
): ShortcodeSuggestion<T> | null {
  const q = fold(code)
  const c = custom.find((e) => fold(e.name) === q)
  if (c) return { kind: 'custom', emoji: c }
  const u = unicode?.find((e) => e.folded.includes(q))
  return u ? { kind: 'unicode', emoji: u, code } : null
}

/** Liste „zuletzt verwendet“ aktualisieren: `key` nach vorn, ohne Duplikate, höchstens `max`. */
export function pushRecent(list: string[], key: string, max = 27): string[] {
  return [key, ...list.filter((k) => k !== key)].slice(0, max)
}
