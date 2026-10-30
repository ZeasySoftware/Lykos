/**
 * Reine Logik der Desktop-Hülle (Titelleiste, Tray) – ohne Tauri, React oder Store, damit testbar mit `npm test`.
 */

/** Was die Titelleiste neben „Lykos“ zeigt. */
export interface TitleContext {
  /** Direktnachrichten-Ansicht aktiv. */
  dmView: boolean
  /** Name der gewählten DM (Anzeigename), falls eine offen ist. */
  dmName?: string | null
  communityName?: string | null
  /** Gewählter Kanal (Text- oder Sprachkanal) der Community. */
  channel?: { name: string; kind: string } | null
}

/**
 * Kontext in der Titelleiste: „Community · #kanal“, „Direktnachrichten · Name“ oder `null` (dann nur „Lykos“).
 */
export function titlebarContext(ctx: TitleContext): string | null {
  if (ctx.dmView) return ctx.dmName ? `Direktnachrichten · ${ctx.dmName}` : 'Direktnachrichten'
  if (!ctx.communityName) return null
  if (!ctx.channel) return ctx.communityName
  const channel = ctx.channel.kind === 'text' ? `#${ctx.channel.name}` : ctx.channel.name
  return `${ctx.communityName} · ${channel}`
}

/** Punkt am Tray-Symbol (muss zu `Badge::parse` in `desktop.rs` passen). */
export type TrayBadge = 'none' | 'unread' | 'mention'

export interface UnreadSummary {
  /** Ungelesene Erwähnungen (Communities, nicht stummgeschaltet) plus DM-Erwähnungen. */
  mentions: number
  /** Gibt es ungelesene Direktnachrichten? */
  dmUnread: boolean
  /** Gibt es Ungelesenes in nicht stummgeschalteten Communities? */
  communityUnread: boolean
}

/** Roter Punkt bei Erwähnungen und DMs (wie bei Discord), heller Punkt bei sonstigem Ungelesenen. */
export function trayBadge(u: UnreadSummary): TrayBadge {
  if (u.mentions > 0 || u.dmUnread) return 'mention'
  if (u.communityUnread) return 'unread'
  return 'none'
}

/** Tooltip des Tray-Symbols. */
export function trayTooltip(u: UnreadSummary, inCall: boolean): string {
  const parts = ['Lykos']
  if (u.mentions > 0) parts.push(u.mentions === 1 ? '1 ungelesene Erwähnung' : `${u.mentions} ungelesene Erwähnungen`)
  else if (u.dmUnread) parts.push('Ungelesene Direktnachrichten')
  else if (u.communityUnread) parts.push('Ungelesene Nachrichten')
  if (inCall) parts.push('Im Sprachkanal')
  return parts.join(' – ')
}

/**
 * Windows 11? `navigator.userAgentData.getHighEntropyValues(['platformVersion'])` liefert unter Windows 11
 * eine Hauptversion ab 13 (Windows 10: 1–10, ältere: 0).
 */
export function isWindows11(platform: string | undefined, platformVersion: string | undefined): boolean {
  if (platform !== 'Windows' || !platformVersion) return false
  const major = Number.parseInt(platformVersion.split('.')[0] ?? '', 10)
  return Number.isFinite(major) && major >= 13
}

/** Verzögerung, bis die Snap-Layouts beim Überfahren von „Maximieren“ erscheinen (wie bei Windows). */
export const SNAP_HOVER_DELAY_MS = 600
