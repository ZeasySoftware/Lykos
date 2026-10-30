import { useEffect } from 'react'
import type { ChosenStatus, PresenceStatus } from './api'

export const presenceLabel: Record<PresenceStatus | 'invisible', string> = {
  online: 'Online',
  idle: 'Abwesend',
  dnd: 'Bitte nicht stören',
  offline: 'Offline',
  invisible: 'Unsichtbar',
}

/** Auswählbare Status (`idle` setzt der Client automatisch). */
export const choosableStatuses: { status: Exclude<ChosenStatus, 'idle'>; hint?: string }[] = [
  { status: 'online' },
  { status: 'dnd', hint: 'Keine Desktop-Benachrichtigungen' },
  { status: 'invisible', hint: 'Für andere offline, du kannst alles nutzen' },
]

const CHOSEN_KEY = 'lykos_status'

export function loadChosenStatus(): ChosenStatus {
  try {
    const v = localStorage.getItem(CHOSEN_KEY)
    if (v === 'online' || v === 'dnd' || v === 'invisible') return v
  } catch {
    // egal
  }
  return 'online'
}

export function saveChosenStatus(status: ChosenStatus) {
  try {
    localStorage.setItem(CHOSEN_KEY, status)
  } catch {
    // egal
  }
}

/** Ausschnitt des Stores, aus dem sich der Status eines Nutzers ergibt. */
export interface PresenceSource {
  user: { id: string } | null
  gatewayStatus: 'connecting' | 'online' | 'offline'
  presences: Record<string, PresenceStatus>
  chosenStatus: ChosenStatus
  autoIdle: boolean
}

/** Angezeigter Status eines Nutzers; für sich selbst aus der eigenen Wahl statt vom Server. */
export function statusOf(s: PresenceSource, userId: string): PresenceStatus {
  if (userId === s.user?.id) {
    if (s.gatewayStatus !== 'online' || s.chosenStatus === 'invisible') return 'offline'
    return s.chosenStatus === 'online' && s.autoIdle ? 'idle' : s.chosenStatus
  }
  return s.presences[userId] ?? 'offline'
}

/** Nach dieser Zeit ohne Eingabe gilt der Nutzer als abwesend. */
export const IDLE_AFTER_MS = 10 * 60 * 1000

/** Ruft `onChange(true)` nach 10 min ohne Maus/Tastatur, `onChange(false)` bei der nächsten Eingabe. */
export function useIdleDetection(onChange: (idle: boolean) => void) {
  useEffect(() => {
    let idle = false
    let timer: number | undefined
    let lastReset = 0

    const arm = () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        idle = true
        onChange(true)
      }, IDLE_AFTER_MS)
    }
    const activity = () => {
      if (idle) {
        idle = false
        onChange(false)
      }
      // Mausbewegungen kommen sehr oft – Timer höchstens einmal pro Sekunde neu stellen.
      const now = Date.now()
      if (now - lastReset < 1000) return
      lastReset = now
      arm()
    }

    const events = ['pointermove', 'pointerdown', 'keydown', 'wheel', 'focus'] as const
    for (const e of events) window.addEventListener(e, activity, { passive: true })
    arm()
    return () => {
      window.clearTimeout(timer)
      for (const e of events) window.removeEventListener(e, activity)
    }
  }, [onChange])
}
