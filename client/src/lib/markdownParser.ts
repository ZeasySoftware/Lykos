/**
 * Kleiner, sicherer Markdown-Parser für Nachrichten. Erzeugt einen Syntaxbaum, den
 * `markdown.tsx` mit React-Elementen rendert – es entsteht nie HTML aus Nutzertext.
 *
 * Unterstützt: **fett**, *kursiv* / _kursiv_, ~~durchgestrichen~~, `Code`, ```Codeblöcke```,
 * > Zitate, ||Spoiler||, automatische Links (nur http/https), @Erwähnungen, Custom-Emotes (`<:name:id>`)
 * und `\` als Maskierung.
 *
 * Beispiele (ausführbar: `npm test`, siehe `markdownParser.test.ts`):
 *   parseInline('**fett** und *kursiv*')
 *     → [bold[text 'fett'], text ' und ', italic[text 'kursiv']]
 *   parseInline('siehe https://example.org/a.')
 *     → [text 'siehe ', link 'https://example.org/a', text '.']
 *   parseBlocks('> Zitat\nAntwort')
 *     → [quote[paragraph[text 'Zitat']], paragraph[text 'Antwort']]
 */

import { parseCustomToken } from './customEmoji.ts'

export type Inline =
  | { type: 'text'; text: string }
  | { type: 'bold' | 'italic' | 'strike' | 'spoiler'; children: Inline[] }
  | { type: 'code'; text: string }
  | { type: 'link'; url: string }
  /** `name` ist der Benutzername so, wie er im Text steht (ohne `@`). */
  | { type: 'mention'; name: string }
  | { type: 'everyone' }
  /** Custom-Emote `<:name:id>` bzw. `<a:name:id>`. */
  | { type: 'emoji'; name: string; id: string; animated: boolean }

export type Block =
  | { type: 'paragraph'; children: Inline[] }
  | { type: 'codeblock'; lang: string | null; text: string }
  | { type: 'quote'; children: Block[] }

/** Maximale Verschachtelungstiefe von Hervorhebungen (schützt vor pathologischen Eingaben). */
const MAX_DEPTH = 6
const ESCAPABLE = new Set(['\\', '*', '_', '~', '|', '`', '>', '@'])

export function isNameChar(c: string | undefined): boolean {
  return c !== undefined && /[A-Za-z0-9_.-]/.test(c)
}

function isWordChar(c: string | undefined): boolean {
  return c !== undefined && /[\p{L}\p{N}]/u.test(c)
}

/** URL-Ende: Satzzeichen am Ende und unausgeglichene Klammern gehören nicht zur URL. */
function trimUrl(raw: string): string {
  let url = raw
  for (;;) {
    const last = url[url.length - 1]
    if (/[.,:;!?'"*~]/.test(last)) url = url.slice(0, -1)
    else if (last === ')' && (url.match(/\(/g)?.length ?? 0) < (url.match(/\)/g)?.length ?? 0)) url = url.slice(0, -1)
    else return url
  }
}

function safeUrl(raw: string): string | null {
  try {
    const u = new URL(raw)
    return u.protocol === 'http:' || u.protocol === 'https:' ? raw : null
  } catch {
    return null
  }
}

/** Zeichenweise Verarbeitung; arbeitet auf Codepunkten, damit Emojis nicht zerteilt werden. */
export function parseInline(text: string, depth = 0): Inline[] {
  const out: Inline[] = []
  let buf = ''
  const flush = () => {
    if (buf) out.push({ type: 'text', text: buf })
    buf = ''
  }
  const push = (node: Inline) => {
    flush()
    out.push(node)
  }

  /** Umschließt `delim` ab `i` einen nicht leeren Inhalt? Liefert das Ende des Inhalts. */
  const closing = (i: number, delim: string, sameLine = false): number => {
    const start = i + delim.length
    let j = text.indexOf(delim, start)
    // Bei `*` nicht das erste Zeichen eines `**` als Ende nehmen.
    while (j !== -1 && delim === '*' && text[j + 1] === '*') j = text.indexOf(delim, j + 2)
    if (j <= start) return -1
    if (sameLine && text.slice(start, j).includes('\n')) return -1
    return j
  }

  let i = 0
  while (i < text.length) {
    const c = text[i]
    const prev = i > 0 ? text[i - 1] : undefined

    if (c === '\\' && ESCAPABLE.has(text[i + 1])) {
      buf += text[i + 1]
      i += 2
      continue
    }

    if (c === '`') {
      const end = closing(i, '`', true)
      if (end !== -1) {
        push({ type: 'code', text: text.slice(i + 1, end) })
        i = end + 1
        continue
      }
    }

    // Custom-Emotes vor den Hervorhebungen, damit `_` in Namen nicht kursiv macht.
    if (c === '<') {
      const token = parseCustomToken(text, i)
      if (token) {
        push({ type: 'emoji', name: token.name, id: token.id, animated: token.animated })
        i += token.length
        continue
      }
    }

    if (depth < MAX_DEPTH) {
      let matched = false
      for (const [delim, type] of [
        ['||', 'spoiler'],
        ['**', 'bold'],
        ['~~', 'strike'],
      ] as const) {
        if (!text.startsWith(delim, i)) continue
        const end = closing(i, delim)
        if (end === -1) continue
        push({ type, children: parseInline(text.slice(i + delim.length, end), depth + 1) })
        i = end + delim.length
        matched = true
        break
      }
      if (matched) continue

      if (c === '*' && text[i + 1] !== '*' && text[i + 1] !== undefined && !/\s/.test(text[i + 1])) {
        const end = closing(i, '*')
        if (end !== -1 && !/\s/.test(text[end - 1])) {
          push({ type: 'italic', children: parseInline(text.slice(i + 1, end), depth + 1) })
          i = end + 1
          continue
        }
      }

      // `_` nur an Wortgrenzen, damit snake_case_namen nicht kursiv werden.
      if (c === '_' && !isWordChar(prev) && text[i + 1] !== '_') {
        let end = closing(i, '_')
        while (end !== -1 && isWordChar(text[end + 1])) end = text.indexOf('_', end + 1)
        if (end > i + 1) {
          push({ type: 'italic', children: parseInline(text.slice(i + 1, end), depth + 1) })
          i = end + 1
          continue
        }
      }
    }

    if ((c === 'h' || c === 'H') && !isWordChar(prev)) {
      const m = /^https?:\/\/[^\s<>]+/i.exec(text.slice(i))
      if (m) {
        const url = safeUrl(trimUrl(m[0]))
        if (url && url.length > m[0].indexOf('//') + 2) {
          push({ type: 'link', url })
          i += url.length
          continue
        }
      }
    }

    if (c === '@' && !isNameChar(prev)) {
      let j = i + 1
      while (j < text.length && j - i <= 33 && isNameChar(text[j])) j++
      const name = text.slice(i + 1, j)
      if (name.toLowerCase() === 'everyone') {
        push({ type: 'everyone' })
        i = j
        continue
      }
      if (name.length >= 2 && name.length <= 32) {
        push({ type: 'mention', name })
        i = j
        continue
      }
    }

    buf += c
    i++
  }
  flush()
  return out
}

/** Text ohne Codeblöcke in Absätze und Zitate gliedern. */
function parseLines(text: string, allowQuotes: boolean): Block[] {
  const blocks: Block[] = []
  const lines = text.split('\n')
  let para: string[] = []
  let quote: string[] = []
  const flushPara = () => {
    if (para.length) blocks.push({ type: 'paragraph', children: parseInline(para.join('\n')) })
    para = []
  }
  const flushQuote = () => {
    if (quote.length) blocks.push({ type: 'quote', children: parseLines(quote.join('\n'), false) })
    quote = []
  }
  for (const line of lines) {
    const q = allowQuotes ? /^> ?(.*)$/.exec(line) : null
    if (q && (line.startsWith('> ') || line === '>')) {
      flushPara()
      quote.push(q[1])
    } else {
      flushQuote()
      para.push(line)
    }
  }
  flushPara()
  flushQuote()
  return blocks
}

/** Gliedert eine Nachricht in Blöcke: Codeblöcke (```), Zitate (> ) und Absätze. */
export function parseBlocks(src: string): Block[] {
  const blocks: Block[] = []
  let rest = src
  for (;;) {
    const open = rest.indexOf('```')
    const close = open === -1 ? -1 : rest.indexOf('```', open + 3)
    if (close === -1) break
    // Ein Zeilenumbruch direkt vor/nach dem Block ist nur Formatierung.
    const before = rest.slice(0, open).replace(/\n$/, '')
    if (before) blocks.push(...parseLines(before, true))
    let body = rest.slice(open + 3, close)
    let lang: string | null = null
    const firstLine = /^([A-Za-z0-9_+#.-]{1,20})\n/.exec(body)
    if (firstLine) {
      lang = firstLine[1]
      body = body.slice(firstLine[0].length)
    } else if (body.startsWith('\n')) {
      body = body.slice(1)
    }
    blocks.push({ type: 'codeblock', lang, text: body.replace(/\n$/, '') })
    rest = rest.slice(close + 3).replace(/^\n/, '')
  }
  if (rest) blocks.push(...parseLines(rest, true))
  return blocks
}

/** Nur Text ohne Markdown-Zeichen, z. B. für Benachrichtigungen und Antwort-Vorschauen. */
export function plainText(src: string): string {
  const inline = (nodes: Inline[]): string =>
    nodes
      .map((n) => {
        switch (n.type) {
          case 'text':
          case 'code':
            return n.text
          case 'link':
            return n.url
          case 'mention':
            return `@${n.name}`
          case 'everyone':
            return '@everyone'
          case 'emoji':
            return `:${n.name}:`
          case 'spoiler':
            return '▒▒▒'
          default:
            return inline(n.children)
        }
      })
      .join('')
  const block = (b: Block): string =>
    b.type === 'paragraph' ? inline(b.children) : b.type === 'codeblock' ? b.text : b.children.map(block).join('\n')
  return parseBlocks(src).map(block).join('\n')
}
