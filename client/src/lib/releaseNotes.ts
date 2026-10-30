/**
 * Release-Notizen (Text der Gitea-Releases) für die Download-Seite. Die Notizen nutzen etwas mehr Markdown als
 * Chat-Nachrichten: Überschriften (`#`–`###`), Listen (`-`/`*`) und benannte Links (`[Text](https://…)`).
 * Innerhalb einer Zeile gilt der sichere Chat-Parser (`parseInline`) – es entsteht nie HTML aus dem Text.
 *
 * Beispiel (ausführbar: `npm test`, siehe `releaseNotes.test.ts`):
 *   parseReleaseNotes('## Neu\n- **Freunde**')
 *     → [heading 2 [text 'Neu'], list [[bold[text 'Freunde']]]]
 */

import { parseInline } from './markdownParser.ts'
import type { Inline } from './markdownParser.ts'

export type NoteInline = Inline | { type: 'named-link'; url: string; children: Inline[] }

export type NoteBlock =
  | { type: 'heading'; level: 1 | 2 | 3; children: NoteInline[] }
  | { type: 'list'; items: NoteInline[][] }
  | { type: 'paragraph'; children: NoteInline[] }

const NAMED_LINK = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g

/** Eine Zeile: benannte Links herauslösen, der Rest läuft durch den Chat-Parser. */
export function parseNoteInline(text: string): NoteInline[] {
  const out: NoteInline[] = []
  let last = 0
  for (const m of text.matchAll(NAMED_LINK)) {
    if (m.index > last) out.push(...parseInline(text.slice(last, m.index)))
    out.push({ type: 'named-link', url: m[2], children: parseInline(m[1]) })
    last = m.index + m[0].length
  }
  if (last < text.length) out.push(...parseInline(text.slice(last)))
  return out
}

export function parseReleaseNotes(src: string): NoteBlock[] {
  const blocks: NoteBlock[] = []
  let paragraph: string[] = []
  let list: NoteInline[][] | null = null

  const flush = () => {
    if (paragraph.length) blocks.push({ type: 'paragraph', children: parseNoteInline(paragraph.join(' ')) })
    paragraph = []
    if (list) blocks.push({ type: 'list', items: list })
    list = null
  }

  for (const raw of src.replace(/\r\n?/g, '\n').split('\n')) {
    const line = raw.trim()
    const heading = /^(#{1,3})\s+(.+)$/.exec(line)
    const item = /^[-*]\s+(.+)$/.exec(line)
    if (!line) {
      flush()
    } else if (heading) {
      flush()
      blocks.push({ type: 'heading', level: heading[1].length as 1 | 2 | 3, children: parseNoteInline(heading[2]) })
    } else if (item) {
      if (paragraph.length) flush()
      list ??= []
      list.push(parseNoteInline(item[1]))
    } else if (list && /^\s{2,}/.test(raw)) {
      // Eingerückte Fortsetzung eines Listenpunkts.
      list[list.length - 1].push({ type: 'text', text: ' ' }, ...parseNoteInline(line))
    } else {
      if (list) flush()
      paragraph.push(line)
    }
  }
  flush()
  return blocks
}
