/**
 * Zustand des Freundesystems: eigene Beziehungen (Freunde, Anfragen, Blockierte). Bewusst getrennt vom
 * App-Store; Gateway-Events (`RELATIONSHIP_ADD`/`RELATIONSHIP_REMOVE`) kommen über `applyAdd`/`applyRemove`,
 * geladen wird nach Anmeldung, Reconnect und RESYNC (`store.ts`).
 */
import { create } from 'zustand'
import { endpoints } from './api'
import type { Relationship, RelationshipType } from './api'
import { friendAddFeedback } from './friends'
import { toast, toastError } from './toast'

interface FriendsState {
  /** user_id → Beziehung. */
  relationships: Record<string, Relationship>
  loaded: boolean

  load: () => Promise<void>
  applyAdd: (r: Relationship) => void
  applyRemove: (userId: string) => void
  reset: () => void

  /** Anfrage per Benutzername; nimmt eine offene Gegenanfrage an. */
  requestByName: (username: string) => Promise<Relationship>
  /** Anfrage senden bzw. annehmen; liefert die neue Beziehung. */
  addFriend: (userId: string) => Promise<Relationship>
  /** Freund entfernen, Anfrage ablehnen/zurückziehen, Blockade aufheben. */
  remove: (userId: string) => Promise<void>
  block: (userId: string) => Promise<void>
}

export const useFriends = create<FriendsState>()((set, get) => ({
  relationships: {},
  loaded: false,

  load: async () => {
    const list = await endpoints.relationships()
    set({ relationships: Object.fromEntries(list.map((r) => [r.id, r])), loaded: true })
  },

  applyAdd: (r) => set((s) => ({ relationships: { ...s.relationships, [r.id]: r } })),

  applyRemove: (userId) =>
    set((s) => {
      if (!(userId in s.relationships)) return {}
      const relationships = { ...s.relationships }
      delete relationships[userId]
      return { relationships }
    }),

  reset: () => set({ relationships: {}, loaded: false }),

  requestByName: async (username) => {
    const r = await endpoints.requestFriend(username)
    get().applyAdd(r)
    return r
  },

  addFriend: async (userId) => {
    const r = await endpoints.putRelationship(userId, 'friend')
    get().applyAdd(r)
    return r
  },

  remove: async (userId) => {
    await endpoints.removeRelationship(userId)
    get().applyRemove(userId)
  },

  block: async (userId) => {
    get().applyAdd(await endpoints.putRelationship(userId, 'blocked'))
  },
}))

/** Art der Beziehung zu einem Nutzer (oder `undefined`). */
export function useRelationshipType(userId: string | undefined): RelationshipType | undefined {
  return useFriends((s) => (userId ? s.relationships[userId]?.type : undefined))
}

/** Hat man den Nutzer blockiert? (Nicht reaktiv – für Menüs und Event-Handler.) */
export function isBlocked(userId: string | undefined): boolean {
  return !!userId && useFriends.getState().relationships[userId]?.type === 'blocked'
}

/**
 * Anfrage senden bzw. annehmen – mit Rückmeldung als Toast (Erfolg wie „Freundschaftsanfrage an … gesendet“,
 * Fehler mit deutschem Text). Gemeinsamer Weg für Profilkarte, Kontextmenüs und Freundesliste; Knöpfe und
 * Menüpunkte zeigen danach über den Store den neuen Zustand („Anfrage zurückziehen“).
 */
export async function addFriendWithFeedback(userId: string, name: string): Promise<void> {
  const s = useFriends.getState()
  const before = s.relationships[userId]?.type
  try {
    const r = await s.addFriend(userId)
    const text = friendAddFeedback(before, r.type, name)
    if (text) toast(text)
  } catch (e) {
    toastError(e)
  }
}
