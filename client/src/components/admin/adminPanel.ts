import { create } from 'zustand'

export type AdminTab = 'overview' | 'invites' | 'users' | 'communities' | 'feedback' | 'settings' | 'audit'

/** Sichtbarkeit der Instanz-Verwaltung – eigener kleiner Store, damit der App-Store unberührt bleibt. */
export const useAdminPanel = create<{
  open: boolean
  tab: AdminTab
  show: (tab?: AdminTab) => void
  hide: () => void
  setTab: (tab: AdminTab) => void
}>((set) => ({
  open: false,
  tab: 'overview',
  show: (tab) => set((s) => ({ open: true, tab: tab ?? s.tab })),
  hide: () => set({ open: false }),
  setTab: (tab) => set({ tab }),
}))
