import { useSyncExternalStore } from 'react'

// Eingeklappte Kategorien – nur lokal auf diesem Gerät (wie bei Discord).

const KEY = 'lykos_collapsed_categories'

function read(): Set<string> {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) ?? '[]') as unknown
    return new Set(Array.isArray(list) ? list.filter((x): x is string => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

let collapsed = read()
let version = 0
const listeners = new Set<() => void>()

export function isCategoryCollapsed(categoryId: string): boolean {
  return collapsed.has(categoryId)
}

export function toggleCategoryCollapsed(categoryId: string) {
  collapsed = new Set(collapsed)
  if (collapsed.has(categoryId)) collapsed.delete(categoryId)
  else collapsed.add(categoryId)
  try {
    localStorage.setItem(KEY, JSON.stringify([...collapsed]))
  } catch {
    // Kein Speicher – dann eben nur bis zum Neuladen.
  }
  version++
  for (const l of listeners) l()
}

/** Neu rendern, wenn sich eingeklappte Kategorien ändern. */
export function useCollapsedVersion(): number {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => version,
  )
}
