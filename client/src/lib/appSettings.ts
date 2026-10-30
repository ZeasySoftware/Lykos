import { create } from 'zustand'
import { initialCloseDecided } from './desktopShell'

/** Lokale App-Einstellungen (nur auf diesem Gerät). */
interface AppSettings {
  /** Entwicklermodus: „ID kopieren“ in den Kontextmenüs (wie bei Discord). */
  developerMode: boolean
  /** Animationen reduzieren – zusätzlich zur Systemeinstellung `prefers-reduced-motion`. */
  reduceMotion: boolean
  /** Desktop: X versteckt das Fenster in den Infobereich statt Lykos zu beenden. */
  closeToTray: boolean
  /** Desktop: Verhalten beim Schließen ist entschieden – sonst fragt Lykos beim X nach. */
  closeDecided: boolean
}

interface AppSettingsStore extends AppSettings {
  update: (patch: Partial<AppSettings>) => void
}

const KEY = 'lykos_app_settings'
const DEFAULTS: AppSettings = { developerMode: false, reduceMotion: false, closeToTray: true, closeDecided: false }

function load(): AppSettings {
  try {
    const stored = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<AppSettings>
    return { ...DEFAULTS, ...stored, closeDecided: initialCloseDecided(stored) }
  } catch {
    return DEFAULTS
  }
}

function applyMotion(reduce: boolean) {
  document.documentElement.classList.toggle('reduce-motion', reduce)
}

export const useAppSettings = create<AppSettingsStore>()((set, get) => {
  const initial = load()
  applyMotion(initial.reduceMotion)
  return {
    ...initial,
    update(patch) {
      set(patch)
      const { developerMode, reduceMotion, closeToTray, closeDecided } = get()
      applyMotion(reduceMotion)
      try {
        localStorage.setItem(KEY, JSON.stringify({ developerMode, reduceMotion, closeToTray, closeDecided }))
      } catch {
        // egal
      }
    },
  }
})
