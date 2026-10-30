import type { Member } from './api'
import type { ResolvedMention } from './markdown'

/** Anzeigename eines Mitglieds (Spitzname vor Anzeigename vor Benutzername). */
export function memberLabel(m: Member): string {
  return m.nickname || m.user.display_name || m.user.username
}

/** Erwähnungen gegen die Mitgliederliste auflösen (Benutzername ohne Groß/Klein-Unterschied). */
export function mentionResolver(
  members: Member[] | undefined,
  selfId: string | undefined,
): (name: string) => ResolvedMention | null {
  const byName = new Map((members ?? []).map((m) => [m.user.username.toLowerCase(), m]))
  return (name) => {
    const m = byName.get(name.toLowerCase())
    return m ? { label: memberLabel(m), self: m.user.id === selfId } : null
  }
}

/** Offene Erwähnung direkt vor dem Cursor: `@que|` → `{ start, query: 'que' }`. */
export function mentionQuery(text: string, caret: number): { start: number; query: string } | null {
  const m = /(^|[^A-Za-z0-9_.-])@([A-Za-z0-9_.-]{0,32})$/.exec(text.slice(0, caret))
  if (!m) return null
  return { start: caret - m[2].length - 1, query: m[2] }
}

export interface MentionSuggestion {
  /** Einzufügender Name ohne `@`. */
  insert: string
  label: string
  detail: string
}

/** Vorschläge für die Autovervollständigung: Präfix-Treffer zuerst, höchstens `limit`. */
export function mentionSuggestions(
  members: Member[] | undefined,
  query: string,
  everyone: boolean,
  limit = 8,
): MentionSuggestion[] {
  const q = query.toLowerCase()
  const scored: { s: MentionSuggestion; rank: number }[] = []
  for (const m of members ?? []) {
    const username = m.user.username.toLowerCase()
    const label = memberLabel(m).toLowerCase()
    const rank = username.startsWith(q) ? 0 : label.startsWith(q) ? 1 : username.includes(q) || label.includes(q) ? 2 : -1
    if (rank >= 0) scored.push({ s: { insert: m.user.username, label: memberLabel(m), detail: m.user.username }, rank })
  }
  scored.sort((a, b) => a.rank - b.rank || a.s.label.localeCompare(b.s.label, 'de'))
  const list = scored.map((x) => x.s)
  if (everyone && 'everyone'.startsWith(q)) {
    list.unshift({ insert: 'everyone', label: 'everyone', detail: 'Alle in diesem Kanal benachrichtigen' })
  }
  return list.slice(0, limit)
}
