/** Berechtigungs-Bits, identisch zum Server (siehe docs/api.md). */
export const Permission = {
  VIEW_CHANNEL: 1 << 0,
  SEND_MESSAGES: 1 << 1,
  MANAGE_MESSAGES: 1 << 2,
  ATTACH_FILES: 1 << 3,
  CONNECT: 1 << 4,
  SPEAK: 1 << 5,
  STREAM: 1 << 6,
  CREATE_INVITES: 1 << 7,
  MANAGE_CHANNELS: 1 << 8,
  MANAGE_ROLES: 1 << 9,
  KICK_MEMBERS: 1 << 10,
  MANAGE_COMMUNITY: 1 << 11,
  VIEW_AUDIT_LOG: 1 << 12,
  VIDEO: 1 << 13,
  ADD_REACTIONS: 1 << 14,
  MENTION_EVERYONE: 1 << 15,
  MANAGE_EMOJIS: 1 << 16,
  ADMINISTRATOR: 1 << 20,
  CHANGE_NICKNAME: 1 << 21,
  MANAGE_NICKNAMES: 1 << 22,
} as const

export type PermissionBit = (typeof Permission)[keyof typeof Permission]

/** Prüft ein Recht; `ADMINISTRATOR` schließt alles ein. */
export function has(permissions: number, bit: PermissionBit): boolean {
  if ((permissions & Permission.ADMINISTRATOR) !== 0) return true
  return (permissions & bit) === bit
}
