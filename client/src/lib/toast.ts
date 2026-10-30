import { create } from 'zustand'
import { errorText } from './errors'

/** Kurze Rückmeldung unten in der Mitte (z. B. „Kopiert“ oder ein Fehler einer Menü-Aktion). */
export interface Toast {
  id: number
  text: string
  error: boolean
}

interface ToastStore {
  toasts: Toast[]
  show: (text: string, error?: boolean) => void
  dismiss: (id: number) => void
}

const DURATION_MS = 2600
let nextId = 1

export const useToasts = create<ToastStore>()((set, get) => ({
  toasts: [],
  show(text, error = false) {
    const id = nextId++
    // Höchstens drei gleichzeitig; gleiche Meldung nicht stapeln.
    set((s) => ({ toasts: [...s.toasts.filter((t) => t.text !== text).slice(-2), { id, text, error }] }))
    window.setTimeout(() => get().dismiss(id), error ? DURATION_MS * 2 : DURATION_MS)
  },
  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }))
  },
}))

export function toast(text: string) {
  useToasts.getState().show(text)
}

export function toastError(e: unknown) {
  useToasts.getState().show(errorText(e), true)
}

/** Führt eine (asynchrone) Aktion aus und zeigt Fehler als Toast. */
export function runAction(action: () => unknown): void {
  try {
    const result = action()
    if (result instanceof Promise) result.catch(toastError)
  } catch (e) {
    toastError(e)
  }
}

/** Text in die Zwischenablage; mit Rückmeldung. */
export async function copyText(text: string, feedback = 'In die Zwischenablage kopiert'): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    // Fallback (z. B. ohne Clipboard-API-Rechte): verstecktes Textfeld.
    const area = document.createElement('textarea')
    area.value = text
    area.style.position = 'fixed'
    area.style.opacity = '0'
    document.body.appendChild(area)
    area.select()
    const ok = document.execCommand('copy')
    area.remove()
    if (!ok) {
      useToasts.getState().show('Kopieren fehlgeschlagen.', true)
      return
    }
  }
  toast(feedback)
}
