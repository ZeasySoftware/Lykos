/**
 * AFK-Kanal: Wer im Sprachkanal einer Community für die eingestellte Zeit weder spricht (Sprachaktivierung bzw.
 * Push-to-Talk) noch die App bedient, wechselt selbst in den AFK-Kanal der Community. Reine Logik, ohne
 * React/Store – die Anbindung steckt in `afkWatcher.ts`.
 */

/** Erlaubte AFK-Zeiten (Sekunden, wie bei Discord) mit Beschriftung. */
export const AFK_TIMEOUTS: { seconds: number; label: string }[] = [
  { seconds: 60, label: '1 Minute' },
  { seconds: 300, label: '5 Minuten' },
  { seconds: 900, label: '15 Minuten' },
  { seconds: 1800, label: '30 Minuten' },
  { seconds: 3600, label: '1 Stunde' },
]

/** Standard des Servers, falls die Community (ältere Server) keinen Wert liefert. */
export const DEFAULT_AFK_TIMEOUT = 300

/** Gültige AFK-Zeit in Sekunden (Unbekanntes → Standard). */
export function afkTimeout(seconds: number | undefined): number {
  return AFK_TIMEOUTS.some((t) => t.seconds === seconds) ? (seconds as number) : DEFAULT_AFK_TIMEOUT
}

/** Beschriftung einer AFK-Zeit, z. B. „5 Minuten“. */
export function afkTimeoutLabel(seconds: number | undefined): string {
  const s = afkTimeout(seconds)
  return AFK_TIMEOUTS.find((t) => t.seconds === s)?.label ?? `${Math.round(s / 60)} Minuten`
}

/**
 * Zeitpunkt der letzten Aktivität: Spricht man gerade (eigene Identity unter den Sprechenden) oder hält man die
 * Push-to-Talk-Taste, zählt „jetzt“ – sonst bleibt der bisherige Wert.
 */
export function nextLastActivity(
  lastActivity: number,
  now: number,
  voice: { speaking: readonly string[]; selfId: string | undefined; pttPressed: boolean },
): number {
  const speaking = !!voice.selfId && voice.speaking.includes(voice.selfId)
  return speaking || voice.pttPressed ? now : lastActivity
}

export interface AfkInput {
  now: number
  /** Letzte Eingabe in der App bzw. letztes Sprechen (ms seit Epoche). */
  lastActivity: number
  /** Verbindungsstatus des eigenen Sprachkanals. */
  status: string
  /** Community des aktuellen Sprachkanals; `null` = DM-Anruf (dort gibt es keinen AFK-Kanal). */
  communityId: string | null
  channelId: string | null
  /** AFK-Kanal der Community (`null`/fehlt = keiner). */
  afkChannelId: string | null | undefined
  afkTimeoutSeconds: number | undefined
  screenSharing: boolean
  cameraOn: boolean
  /** Darf man den AFK-Kanal betreten (sehen + `CONNECT`)? */
  canJoinAfk: boolean
}

/** Soll der Client jetzt in den AFK-Kanal wechseln? Nie beim Streamen oder mit Kamera, nie aus DM-Anrufen. */
export function shouldMoveToAfk(i: AfkInput): boolean {
  if (i.status !== 'connected' || !i.communityId || !i.channelId) return false
  if (!i.afkChannelId || i.afkChannelId === i.channelId || !i.canJoinAfk) return false
  if (i.screenSharing || i.cameraOn) return false
  return i.now - i.lastActivity >= afkTimeout(i.afkTimeoutSeconds) * 1000
}
