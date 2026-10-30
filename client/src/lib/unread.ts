import type { CommunityDetail, DmChannel } from './api'

/** IDs sind UUIDv7 und damit als Zeichenkette zeitlich sortierbar. */
export function isNewer(a: string, b: string | undefined): boolean {
  return b === undefined || a > b
}

/** Hat der Textkanal Nachrichten nach dem eigenen Lesestand? */
export function isChannelUnread(detail: CommunityDetail | undefined, channelId: string): boolean {
  const last = detail?.last_message_ids?.[channelId]
  return !!last && isNewer(last, detail?.read_states?.[channelId])
}

/** Summe der ungelesenen Erwähnungen über alle sichtbaren Textkanäle der Community. */
export function communityMentionCount(detail: CommunityDetail | undefined): number {
  if (!detail?.mention_counts) return 0
  return detail.channels.reduce((sum, c) => sum + (detail.mention_counts?.[c.id] ?? 0), 0)
}

/** Hat die DM Nachrichten nach dem eigenen Lesestand? */
export function isDmUnread(dm: DmChannel | undefined): boolean {
  return !!dm?.last_message_id && isNewer(dm.last_message_id, dm.last_read_message_id ?? undefined)
}

/** Hat irgendein sichtbarer Textkanal der Community Ungelesenes? */
export function isCommunityUnread(detail: CommunityDetail | undefined): boolean {
  if (!detail) return false
  return detail.channels.some((c) => c.kind === 'text' && isChannelUnread(detail, c.id))
}
