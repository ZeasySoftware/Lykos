import { useSyncExternalStore } from 'react'
import { isDesktopBundle } from './api'

// ---------------------------------------------------------------------------
// Stummgeschaltete Kanäle und Communities (nur lokal auf diesem Gerät)
// ---------------------------------------------------------------------------

/** Früheres Format: Liste von Kanal-IDs (unbefristet). */
const LEGACY_KEY = 'lykos_muted_channels'
/** `{ [id]: bis (ms) | 0 = bis ich es ändere }`; Communities mit Präfix `c:`. */
const MUTES_KEY = 'lykos_mutes'

/** Dauer-Auswahl wie bei Discord; `null` = bis ich es ändere. */
export const MUTE_DURATIONS: { label: string; ms: number | null }[] = [
  { label: 'Für 15 Minuten', ms: 15 * 60_000 },
  { label: 'Für 1 Stunde', ms: 60 * 60_000 },
  { label: 'Für 8 Stunden', ms: 8 * 60 * 60_000 },
  { label: 'Für 24 Stunden', ms: 24 * 60 * 60_000 },
  { label: 'Bis ich es ändere', ms: null },
]

type Mutes = Record<string, number>

function readMutes(): Mutes {
  try {
    const raw = localStorage.getItem(MUTES_KEY)
    if (raw) return JSON.parse(raw) as Mutes
    const legacy = JSON.parse(localStorage.getItem(LEGACY_KEY) ?? '[]') as string[]
    return Object.fromEntries(legacy.map((id) => [id, 0]))
  } catch {
    return {}
  }
}

let mutes = readMutes()
let version = 0
const listeners = new Set<() => void>()
let expiryTimer: number | undefined

function changed() {
  version++
  for (const l of listeners) l()
  scheduleExpiry()
}

function save() {
  try {
    localStorage.setItem(MUTES_KEY, JSON.stringify(mutes))
  } catch {
    // egal
  }
}

/** Abgelaufene Stummschaltungen entfernen und zum nächsten Ablauf neu planen. */
function scheduleExpiry() {
  window.clearTimeout(expiryTimer)
  const now = Date.now()
  const pending = Object.values(mutes).filter((until) => until > 0)
  if (pending.length === 0) return
  const next = Math.min(...pending)
  expiryTimer = window.setTimeout(
    () => {
      const t = Date.now()
      const expired = Object.entries(mutes).filter(([, until]) => until > 0 && until <= t)
      if (expired.length === 0) return scheduleExpiry()
      mutes = { ...mutes }
      for (const [id] of expired) delete mutes[id]
      save()
      changed()
    },
    Math.min(Math.max(0, next - now) + 50, 2 ** 31 - 1),
  )
}
scheduleExpiry()

function mutedUntil(id: string): number | null | undefined {
  const until = mutes[id]
  if (until === undefined) return undefined
  if (until > 0 && until <= Date.now()) return undefined
  return until === 0 ? null : until
}

function setMute(id: string, durationMs: number | null | false) {
  mutes = { ...mutes }
  if (durationMs === false) delete mutes[id]
  else mutes[id] = durationMs === null ? 0 : Date.now() + durationMs
  save()
  changed()
}

export function isChannelMuted(channelId: string): boolean {
  return mutedUntil(channelId) !== undefined
}

export function isCommunityMuted(communityId: string): boolean {
  return mutedUntil(`c:${communityId}`) !== undefined
}

/** `false` hebt die Stummschaltung auf, `null` = unbefristet, sonst Dauer in ms. */
export function setChannelMuted(channelId: string, duration: number | null | boolean) {
  setMute(channelId, duration === true ? null : duration)
}

export function setCommunityMuted(communityId: string, duration: number | null | boolean) {
  setMute(`c:${communityId}`, duration === true ? null : duration)
}

/** Ende der Stummschaltung als Text („bis 14:30“) oder `null`, wenn unbefristet/nicht stumm. */
export function muteUntilLabel(id: { channelId?: string; communityId?: string }): string | null {
  const until = id.channelId ? mutedUntil(id.channelId) : mutedUntil(`c:${id.communityId}`)
  if (!until) return null
  const d = new Date(until)
  const sameDay = d.toDateString() === new Date().toDateString()
  const fmt = new Intl.DateTimeFormat('de-DE', sameDay ? { timeStyle: 'short' } : { dateStyle: 'short', timeStyle: 'short' })
  return `Stumm bis ${fmt.format(d)}`
}

/** Alle aktiven Stummschaltungen (für die Übersicht in den Einstellungen). */
export function listMutes(): { kind: 'channel' | 'community'; id: string; label: string | null }[] {
  return Object.keys(mutes)
    .filter((key) => mutedUntil(key) !== undefined)
    .map((key) =>
      key.startsWith('c:')
        ? { kind: 'community' as const, id: key.slice(2), label: muteUntilLabel({ communityId: key.slice(2) }) }
        : { kind: 'channel' as const, id: key, label: muteUntilLabel({ channelId: key }) },
    )
}

/** Bei jeder Änderung der Stummschaltungen benachrichtigen (außerhalb von React). Liefert die Abmeldung. */
export function subscribeMutes(cb: () => void): () => void {
  listeners.add(cb)
  return () => listeners.delete(cb)
}

/** Re-Render bei jeder Änderung der Stummschaltungen. */
export function useMutesVersion(): number {
  return useSyncExternalStore(subscribeMutes, () => version)
}

// ---------------------------------------------------------------------------
// Desktop-Benachrichtigungen (nur im Desktop-Bundle, über tauri-plugin-notification)
// ---------------------------------------------------------------------------

let permission: Promise<boolean> | null = null

async function ensurePermission(): Promise<boolean> {
  const n = await import('@tauri-apps/plugin-notification')
  if (await n.isPermissionGranted()) return true
  return (await n.requestPermission()) === 'granted'
}

/** Zeigt eine Desktop-Benachrichtigung; im Browser ohne Wirkung. */
export async function notify(title: string, body: string): Promise<void> {
  if (!isDesktopBundle) return
  try {
    permission ??= ensurePermission()
    if (!(await permission)) return
    const n = await import('@tauri-apps/plugin-notification')
    n.sendNotification({ title, body })
  } catch (e) {
    permission = null
    console.warn('Benachrichtigung fehlgeschlagen', e)
  }
}
