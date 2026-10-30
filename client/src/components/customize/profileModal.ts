import { create } from 'zustand'

interface ProfileModalState {
  target: { userId: string; communityId: string | null } | null
  open: (userId: string, communityId: string | null) => void
  close: () => void
}

/** Geöffnetes Profil-Modal (eigener kleiner Store, damit der App-Store unberührt bleibt). */
export const useProfileModal = create<ProfileModalState>((set) => ({
  target: null,
  open: (userId, communityId) => set({ target: { userId, communityId } }),
  close: () => set({ target: null }),
}))
