/**
 * Zustand der Feedback-Oberfläche (Paket K): welches Fenster offen ist und die eigenen Meldungen.
 * Bewusst getrennt vom App-Store; Gateway-Events (`FEEDBACK_UPDATE`/`FEEDBACK_DELETE`) kommen über
 * `applyUpdate`/`applyDelete`.
 */
import { create } from 'zustand'
import { feedbackEndpoints } from './feedbackApi'
import type { Feedback, FeedbackKind } from './feedbackApi'

export type FeedbackPanel = 'new' | 'mine' | 'suggestions'

interface FeedbackState {
  /** Offenes Feedback-Fenster (Reiter) oder `null`. */
  panel: FeedbackPanel | null
  /** Vorauswahl des Typs beim Öffnen von „Melden“. */
  initialKind: FeedbackKind
  /** Zählt Öffnungen mit Typ-Vorgabe, damit ein offenes Formular den Typ übernimmt. */
  kindRequest: number
  /** Eigene Meldungen, neueste zuerst; `null` = noch nicht geladen. */
  mine: Feedback[] | null
  /** IDs eigener Meldungen mit ungelesener Antwort/Statusänderung. */
  unseen: string[]

  open: (panel?: FeedbackPanel, kind?: FeedbackKind) => void
  close: () => void
  loadMine: () => Promise<void>
  /** Eben abgesendete oder aktualisierte eigene Meldung übernehmen. */
  upsertMine: (f: Feedback) => void
  applyUpdate: (f: Feedback) => void
  applyDelete: (id: string) => void
  markSeen: () => void
  reset: () => void
}

export const useFeedback = create<FeedbackState>()((set, get) => ({
  panel: null,
  initialKind: 'bug',
  kindRequest: 0,
  mine: null,
  unseen: [],

  open: (panel = 'new', kind) =>
    set((s) => (kind ? { panel, initialKind: kind, kindRequest: s.kindRequest + 1 } : { panel })),
  close: () => set({ panel: null }),

  loadMine: async () => {
    const mine = await feedbackEndpoints.mine()
    set({ mine })
  },

  upsertMine: (f) =>
    set((s) => {
      if (!s.mine) return {}
      const exists = s.mine.some((m) => m.id === f.id)
      return { mine: exists ? s.mine.map((m) => (m.id === f.id ? f : m)) : [f, ...s.mine] }
    }),

  applyUpdate: (f) => {
    get().upsertMine(f)
    // Gerade geöffnet → gilt als gesehen.
    if (get().panel === 'mine') return
    set((s) => ({ unseen: s.unseen.includes(f.id) ? s.unseen : [...s.unseen, f.id] }))
  },

  applyDelete: (id) =>
    set((s) => ({
      mine: s.mine?.filter((m) => m.id !== id) ?? null,
      unseen: s.unseen.filter((u) => u !== id),
    })),

  markSeen: () => set({ unseen: [] }),
  reset: () => set({ panel: null, mine: null, unseen: [] }),
}))
