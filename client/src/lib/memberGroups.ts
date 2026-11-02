// Gruppierung der Mitgliederliste: Online-Mitglieder stehen unter ihrer höchsten Rolle (in Hierarchie-Reihenfolge),
// wer nur „@alle“ hat, unter „Online“. Offline-Mitglieder bilden eine Gruppe, innerhalb nach Rolle geordnet. Bots
// stehen unabhängig vom Status in einer eigenen Gruppe. Leere Gruppen entfallen.
import type { Role } from './api'
import { sortRoles } from './roles.ts'

export interface GroupableMember {
  user: { id: string; bot?: boolean }
  role_ids: string[]
}

export interface MemberGroup<M> {
  /** Stabiler Schlüssel (Rollen-ID bzw. `online`/`offline`/`bots`). */
  key: string
  title: string
  /** Rollenfarbe der Gruppe (0xRRGGBB), sonst `null`. */
  color: number | null
  items: M[]
}

/**
 * `sorted`: Mitglieder in Anzeige-Reihenfolge (z. B. nach Name); die Reihenfolge innerhalb einer Gruppe bleibt
 * erhalten. `online`: IDs der Mitglieder, die nicht offline sind.
 */
export function groupMembers<M extends GroupableMember>(
  sorted: M[],
  online: ReadonlySet<string>,
  roles: Role[],
): MemberGroup<M>[] {
  const ranked = sortRoles(roles).filter((r) => !r.is_default)
  const rank = new Map(ranked.map((r, i) => [r.id, i]))
  /** Rang der höchsten Rolle; ohne (Nicht-Standard-)Rolle ganz hinten. */
  const topRank = (m: M) => {
    let best = ranked.length
    for (const id of m.role_ids) {
      const r = rank.get(id)
      if (r !== undefined && r < best) best = r
    }
    return best
  }

  const byRole: M[][] = ranked.map(() => [])
  const noRole: M[] = []
  const offline: M[] = []
  const bots: M[] = []
  for (const m of sorted) {
    if (m.user.bot) bots.push(m)
    else if (!online.has(m.user.id)) offline.push(m)
    else {
      const r = topRank(m)
      if (r < ranked.length) byRole[r].push(m)
      else noRole.push(m)
    }
  }
  // Stabil sortieren: gleiche Rolle behält die Eingangsreihenfolge.
  const offlineRanked = offline.map((m, i) => ({ m, r: topRank(m), i }))
  offlineRanked.sort((a, b) => a.r - b.r || a.i - b.i)

  const groups: MemberGroup<M>[] = ranked.map((role, i) => ({
    key: role.id,
    title: role.name,
    color: role.color,
    items: byRole[i],
  }))
  groups.push(
    { key: 'online', title: 'Online', color: null, items: noRole },
    { key: 'offline', title: 'Offline', color: null, items: offlineRanked.map((x) => x.m) },
    { key: 'bots', title: 'Bots', color: null, items: bots },
  )
  return groups.filter((g) => g.items.length > 0)
}
