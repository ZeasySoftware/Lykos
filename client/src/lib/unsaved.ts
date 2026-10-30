import { createContext, useContext, useEffect, useState } from 'react'

/**
 * Ungespeicherte Änderungen in Einstellungs-Formularen (wie bei Discord): Formulare melden ihren
 * Zustand an die umgebende Einstellungsebene; die zeigt unten die Leiste „Zurücksetzen / Änderungen
 * speichern“ und verhindert Schließen/Bereichswechsel, solange etwas offen ist.
 */
export interface PendingChanges {
  dirty: boolean
  /** Speichern läuft. */
  busy?: boolean
  save: () => unknown
  reset: () => void
}

interface UnsavedApi {
  report: (form: object, pending: PendingChanges | null) => void
  /** `true` (und Leiste hervorheben), wenn gerade ungespeicherte Änderungen offen sind. */
  blocked: () => boolean
}

export const UnsavedContext = createContext<UnsavedApi>({ report: () => {}, blocked: () => false })

/** Formularzustand an die Einstellungsebene melden (bei jedem Rendern aktuell). */
export function useUnsavedChanges(pending: PendingChanges) {
  const { report } = useContext(UnsavedContext)
  const [form] = useState(() => ({}))
  useEffect(() => {
    report(form, pending.dirty ? pending : null)
  })
  useEffect(() => () => report(form, null), [report, form])
}

/**
 * Für Auswahl-Wechsel innerhalb eines Bereichs (z. B. andere Rolle wählen): liefert eine Funktion,
 * die bei offenen Änderungen `true` zurückgibt und die Leiste hervorhebt.
 */
export function useUnsavedGuard(): () => boolean {
  return useContext(UnsavedContext).blocked
}
