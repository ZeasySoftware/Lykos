import type { Role } from './api'

/**
 * Rollen in Hierarchie-Reihenfolge: kleinere `position` = höher. Die Standardrolle „@alle“
 * steht immer ganz unten (siehe docs/api.md → Rollenhierarchie).
 */
export function sortRoles(roles: Role[]): Role[] {
  return roles.slice().sort((a, b) => {
    if (a.is_default !== b.is_default) return a.is_default ? 1 : -1
    return a.position - b.position || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  })
}

/**
 * Höchste eigene Rolle (kleinste Position, ohne Standardrolle); `null` = nur „@alle“.
 * `undefined`, solange die eigenen Rollen unbekannt sind.
 */
export function topPosition(roles: Role[], myRoleIds: string[] | undefined): number | null | undefined {
  if (!myRoleIds) return undefined
  const positions = roles.filter((r) => !r.is_default && myRoleIds.includes(r.id)).map((r) => r.position)
  return positions.length ? Math.min(...positions) : null
}

/** Darf ich diese Rolle verwalten (bearbeiten, löschen, verschieben, zuweisen)? Spiegel der Server-Prüfung. */
export function outranks(role: Role, isOwner: boolean, top: number | null | undefined): boolean {
  if (isOwner || role.is_default) return true
  return top != null && role.position > top
}
