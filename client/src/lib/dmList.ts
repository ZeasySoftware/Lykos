/**
 * DM-Seitenleiste – reine Logik ohne Store/DOM (Unit-Tests: `dmList.test.ts`): welche Unterhaltungen erscheinen
 * und in welcher Reihenfolge.
 */
import type { DmChannel } from './api'

/** Höchstzahl angepinnter Unterhaltungen, wie auf dem Server. */
export const MAX_DM_PINS = 20

/** Neueste Aktivität (UUIDv7: letzte Nachricht bzw. Erstellung) – lexikografisch vergleichbar. */
export function dmActivity(dm: Pick<DmChannel, 'id' | 'last_message_id'>): string {
  return dm.last_message_id ?? dm.id
}

export interface DmSections {
  /** Angepinnte Unterhaltungen, zuletzt angepinnte oben. */
  pinned: DmChannel[]
  /** Übrige Unterhaltungen, neueste Aktivität zuerst. */
  recent: DmChannel[]
}

/**
 * Teilt die DMs in „Angepinnt“ und den Rest. Leere 1:1-DMs (nur geöffnet, nie geschrieben) erscheinen erst mit
 * Nachrichten – außer sie sind angepinnt, ausgewählt oder es läuft ein Anruf. Gruppen erscheinen immer.
 */
export function dmSections(
  dms: Iterable<DmChannel>,
  opts: { selectedId?: string | null; callIds?: ReadonlySet<string> | Record<string, unknown> } = {},
): DmSections {
  const { selectedId = null, callIds } = opts
  const inCall = (id: string) => (callIds instanceof Set ? callIds.has(id) : !!callIds && id in callIds)
  const pinned: DmChannel[] = []
  const recent: DmChannel[] = []
  for (const dm of dms) {
    if (dm.pinned_at) pinned.push(dm)
    else if (dm.kind === 'group_dm' || dm.last_message_id || dm.id === selectedId || inCall(dm.id)) recent.push(dm)
  }
  // Angepinnt: nach Anpinn-Zeitpunkt, zuletzt Angepinntes oben (wie in gängigen Messengern) – neue Nachrichten
  // lassen die Liste nicht springen; bei Gleichstand nach Aktivität.
  pinned.sort((a, b) => {
    const ta = Date.parse(a.pinned_at!)
    const tb = Date.parse(b.pinned_at!)
    if (ta !== tb) return tb - ta
    return dmActivity(a) < dmActivity(b) ? 1 : -1
  })
  recent.sort((a, b) => (dmActivity(a) < dmActivity(b) ? 1 : dmActivity(a) > dmActivity(b) ? -1 : 0))
  return { pinned, recent }
}

/** Personen, die man zu einer Unterhaltung hinzufügen kann: Kontakte ohne die bereits enthaltenen. */
export function addableContacts<T extends { id: string }>(contacts: T[], recipients: { id: string }[]): T[] {
  const inside = new Set(recipients.map((u) => u.id))
  return contacts.filter((u) => !inside.has(u.id))
}

/** Namen als Aufzählung: „Anna“, „Anna und Bert“, „Anna, Bert und Carl“. */
export function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? ''
  return `${names.slice(0, -1).join(', ')} und ${names[names.length - 1]}`
}

/** Wie viele Personen kommen noch dazu (Gruppen bis 10 inkl. einem selbst)? */
export function remainingSlots(recipientCount: number, maxGroupSize = 10): number {
  return Math.max(0, maxGroupSize - recipientCount)
}
