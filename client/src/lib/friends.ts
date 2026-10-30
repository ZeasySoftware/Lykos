/**
 * Freundesystem – reine Logik ohne Store/DOM (Unit-Tests: `friends.test.ts`): Tabs der Freundesliste,
 * Suche, Sortierung, verfügbare Aktionen je Beziehung und das Zusammenfassen von Nachrichten blockierter
 * Nutzer in der Nachrichtenansicht.
 */
import type { PresenceStatus, Relationship, RelationshipType } from './api'

export type FriendsTab = 'online' | 'all' | 'pending' | 'blocked'

export const FRIENDS_TABS: { id: FriendsTab; label: string }[] = [
  { id: 'online', label: 'Online' },
  { id: 'all', label: 'Alle' },
  { id: 'pending', label: 'Ausstehend' },
  { id: 'blocked', label: 'Blockiert' },
]

function nameOf(r: Relationship): string {
  return r.user.display_name || r.user.username
}

/** Passt die Beziehung zur Suche (Benutzername oder Anzeigename, ohne Groß-/Kleinschreibung)? */
export function matchesQuery(r: Relationship, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return r.user.username.toLowerCase().includes(q) || !!r.user.display_name?.toLowerCase().includes(q)
}

/**
 * Einträge eines Tabs: Online = Freunde, die nicht offline sind; Alle = alle Freunde; Ausstehend =
 * eingehende vor ausgehenden Anfragen; Blockiert. Jeweils nach Anzeigename sortiert.
 */
export function friendsInTab(
  list: Relationship[],
  tab: FriendsTab,
  query: string,
  statusOf: (userId: string) => PresenceStatus,
): Relationship[] {
  const wanted = (r: Relationship): boolean => {
    switch (tab) {
      case 'online':
        return r.type === 'friend' && statusOf(r.id) !== 'offline'
      case 'all':
        return r.type === 'friend'
      case 'pending':
        return r.type === 'pending_in' || r.type === 'pending_out'
      case 'blocked':
        return r.type === 'blocked'
    }
  }
  const rank = (r: Relationship) => (r.type === 'pending_in' ? 0 : 1)
  return list
    .filter((r) => wanted(r) && matchesQuery(r, query))
    .sort((a, b) => rank(a) - rank(b) || nameOf(a).localeCompare(nameOf(b), 'de', { sensitivity: 'base' }))
}

/** Anzahl offener eingehender Anfragen (Badge an der DM-Kachel und am Eintrag „Freunde“). */
export function incomingCount(list: Iterable<Relationship>): number {
  let n = 0
  for (const r of list) if (r.type === 'pending_in') n++
  return n
}

export interface FriendActions {
  /** Anfrage senden. */
  add: boolean
  accept: boolean
  decline: boolean
  /** Eigene Anfrage zurückziehen. */
  cancel: boolean
  /** Freundschaft beenden. */
  remove: boolean
  block: boolean
  unblock: boolean
}

/** Welche Aktionen gibt es gegenüber einem Nutzer mit dieser Beziehung (`undefined` = keine)? */
export function friendActions(type: RelationshipType | undefined, isSelf: boolean): FriendActions {
  const none: FriendActions = {
    add: false,
    accept: false,
    decline: false,
    cancel: false,
    remove: false,
    block: false,
    unblock: false,
  }
  if (isSelf) return none
  return {
    add: type === undefined,
    accept: type === 'pending_in',
    decline: type === 'pending_in',
    cancel: type === 'pending_out',
    remove: type === 'friend',
    block: type !== 'blocked',
    unblock: type === 'blocked',
  }
}

/**
 * Rückmeldung nach „Freund hinzufügen“ bzw. „Anfrage annehmen“ (Toast und Formular „Freund hinzufügen“):
 * `before` ist die Beziehung vor der Aktion, `after` die Antwort des Servers. Eine offene Gegenanfrage nimmt
 * der Server direkt an – dann seid ihr sofort befreundet. `null` = keine Meldung nötig.
 */
export function friendAddFeedback(
  before: RelationshipType | undefined,
  after: RelationshipType,
  name: string,
): string | null {
  if (after === 'pending_out') return `Freundschaftsanfrage an ${name} gesendet.`
  if (after !== 'friend' || before === 'friend') return null
  return before === 'pending_in'
    ? `Du und ${name} seid jetzt befreundet.`
    : `Ihr seid jetzt befreundet – ${name} hatte dich schon angefragt.`
}

/**
 * Zeigt der Server das volle Profil über die Beziehung? Nur Freunden und bei eingehender Anfrage – eine eigene
 * Anfrage oder Blockade öffnet das Profil eines Fremden nicht (gemeinsame Community/DM aber weiterhin).
 */
export function relationshipShowsProfile(type: RelationshipType): boolean {
  return type === 'friend' || type === 'pending_in'
}

/**
 * Zusammenhängende Nachrichten blockierter Autoren: Startindex → Anzahl. Die Nachrichtenansicht zeigt an
 * jedem Start einen Platzhalter („2 Nachrichten von blockierten Nutzern – Anzeigen“) und blendet den
 * Rest des Laufs aus.
 */
export function blockedRuns<T extends { author?: { id: string } | null }>(
  items: T[],
  isBlocked: (userId: string) => boolean,
): Map<number, number> {
  const runs = new Map<number, number>()
  let start = -1
  items.forEach((m, i) => {
    const blocked = !!m.author && isBlocked(m.author.id)
    if (blocked) {
      if (start < 0) start = i
      runs.set(start, i - start + 1)
    } else {
      start = -1
    }
  })
  return runs
}

/** Gehört Index `i` zu einem Lauf blockierter Nachrichten? Liefert dann dessen Startindex. */
export function blockedRunStart(runs: Map<number, number>, i: number): number | null {
  for (const [start, len] of runs) if (i >= start && i < start + len) return start
  return null
}

/** Beschriftung des Platzhalters für ausgeblendete Nachrichten. */
export function blockedLabel(count: number): string {
  return count === 1 ? '1 Nachricht von einem blockierten Nutzer' : `${count} Nachrichten von blockierten Nutzern`
}
