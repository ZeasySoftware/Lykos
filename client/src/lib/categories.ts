// Gruppierung der Kanalliste nach Kategorien (reine Logik, getestet in categories.test.ts).
import type { Category, Channel } from './api'

export interface ChannelSection {
  text: Channel[]
  voice: Channel[]
}

export interface CategorySection extends ChannelSection {
  category: Category
}

export interface SidebarLayout {
  /** Kanäle ohne Kategorie – oben unter „Textkanäle“/„Sprachkanäle“ wie bisher. */
  uncategorized: ChannelSection
  /** Kategorien nach `position`, darin Text vor Sprache nach `position`. */
  categories: CategorySection[]
}

const byPosition = <T extends { position: number; id: string }>(a: T, b: T) =>
  a.position - b.position || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)

/**
 * Seitenleiste aufbauen. Kanäle mit unbekannter Kategorie stehen oben bei den übrigen. Eine Kategorie
 * erscheint nur mit mindestens einem (sichtbaren) Kanal – außer für Leute mit `MANAGE_CHANNELS`
 * (`showEmpty`), die leere Kategorien befüllen können sollen. Der Server filtert genauso; der Client
 * prüft erneut, weil Kanäle per Event verschwinden können.
 */
export function sidebarLayout(
  channels: Channel[],
  categories: Category[] | undefined,
  showEmpty: boolean,
): SidebarLayout {
  const known = new Map((categories ?? []).map((c) => [c.id, c]))
  const sorted = [...channels].sort(byPosition)
  const section = (list: Channel[]): ChannelSection => ({
    text: list.filter((c) => c.kind === 'text'),
    voice: list.filter((c) => c.kind === 'voice'),
  })
  const inCategory = new Map<string, Channel[]>()
  const rest: Channel[] = []
  for (const channel of sorted) {
    const id = channel.category_id
    if (id && known.has(id)) {
      const list = inCategory.get(id) ?? []
      list.push(channel)
      inCategory.set(id, list)
    } else {
      rest.push(channel)
    }
  }
  return {
    uncategorized: section(rest),
    categories: [...known.values()]
      .sort(byPosition)
      .filter((c) => showEmpty || (inCategory.get(c.id)?.length ?? 0) > 0)
      .map((category) => ({ category, ...section(inCategory.get(category.id) ?? []) })),
  }
}

/** Kategorien einer Community in Anzeige-Reihenfolge (für Auswahlfelder). */
export function sortedCategories(categories: Category[] | undefined): Category[] {
  return [...(categories ?? [])].sort(byPosition)
}

/** Kategorie anlegen/ersetzen (Event `CATEGORY_CREATE`/`CATEGORY_UPDATE`). */
export function upsertCategory(categories: Category[] | undefined, category: Category): Category[] {
  const list = categories ?? []
  return list.some((c) => c.id === category.id)
    ? list.map((c) => (c.id === category.id ? category : c))
    : [...list, category]
}

/** Gehört die Kategorie-ID eines Kanals zu einer bekannten Kategorie (oder hat er keine)? */
export function knowsCategory(categories: Category[] | undefined, categoryId: string | null | undefined): boolean {
  return !categoryId || (categories ?? []).some((c) => c.id === categoryId)
}
