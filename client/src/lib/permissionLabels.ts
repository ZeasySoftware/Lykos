import { Permission } from './permissions'
import type { ChannelKind } from './api'

export interface PermissionInfo {
  bit: number
  label: string
  description: string
  /** In welchen Kanälen das Recht per Override sinnvoll ist; `null` = nur Community-weit. */
  channels: ChannelKind[] | null
}

const BOTH: ChannelKind[] = ['text', 'voice']

/** Alle Rechte mit deutschen Beschriftungen, in Anzeigereihenfolge. */
export const PERMISSIONS: PermissionInfo[] = [
  {
    bit: Permission.VIEW_CHANNEL,
    label: 'Kanäle sehen',
    description: 'Kanal sehen und Verlauf lesen.',
    channels: BOTH,
  },
  { bit: Permission.SEND_MESSAGES, label: 'Nachrichten senden', description: 'In Textkanälen schreiben und antworten.', channels: ['text'] },
  {
    bit: Permission.MANAGE_MESSAGES,
    label: 'Nachrichten verwalten',
    description: 'Nachrichten anderer löschen, Nachrichten anpinnen.',
    channels: ['text'],
  },
  { bit: Permission.ATTACH_FILES, label: 'Dateien anhängen', description: 'Bilder und Dateien an Nachrichten hängen.', channels: ['text'] },
  { bit: Permission.ADD_REACTIONS, label: 'Reaktionen hinzufügen', description: 'Mit Emojis auf Nachrichten reagieren.', channels: ['text'] },
  {
    bit: Permission.MENTION_EVERYONE,
    label: '@everyone erwähnen',
    description: 'Mit @everyone alle Mitglieder benachrichtigen.',
    channels: ['text'],
  },
  { bit: Permission.CONNECT, label: 'Sprachkanälen beitreten', description: 'Sprachkanäle betreten und zuhören.', channels: ['voice'] },
  { bit: Permission.SPEAK, label: 'Sprechen', description: 'Im Sprachkanal das Mikrofon nutzen.', channels: ['voice'] },
  { bit: Permission.STREAM, label: 'Bildschirm übertragen', description: 'Bildschirm oder Fenster im Sprachkanal teilen.', channels: ['voice'] },
  { bit: Permission.VIDEO, label: 'Kamera nutzen', description: 'Webcam in Sprachkanälen einschalten.', channels: ['voice'] },
  { bit: Permission.CREATE_INVITES, label: 'Einladungen erstellen', description: 'Einladungslinks für neue Mitglieder erstellen.', channels: null },
  {
    bit: Permission.MANAGE_CHANNELS,
    label: 'Kanäle verwalten',
    description: 'Kanäle anlegen, umbenennen und löschen.',
    channels: BOTH,
  },
  {
    bit: Permission.MANAGE_ROLES,
    label: 'Rollen verwalten',
    description: 'Rollen, Rollenzuweisungen und Kanal-Berechtigungen.',
    channels: BOTH,
  },
  { bit: Permission.KICK_MEMBERS, label: 'Mitglieder entfernen', description: 'Mitglieder aus der Community werfen; mit einer neuen Einladung können sie zurückkommen.', channels: null },
  {
    bit: Permission.BAN_MEMBERS,
    label: 'Mitglieder bannen',
    description: 'Mitglieder dauerhaft ausschließen, Banns aufheben und die Bannliste sehen. Gebannte können über keine Einladung zurückkommen.',
    channels: null,
  },
  {
    bit: Permission.MANAGE_EMOJIS,
    label: 'Emojis verwalten',
    description: 'Eigene Emojis der Community hochladen, umbenennen und löschen.',
    channels: null,
  },
  {
    bit: Permission.MANAGE_WEBHOOKS,
    label: 'Webhooks verwalten',
    description: 'Webhooks für Integrationen anlegen, ändern, ihre URL neu erzeugen und löschen.',
    channels: ['text'],
  },
  {
    bit: Permission.CHANGE_NICKNAME,
    label: 'Nickname ändern',
    description: 'Eigenen Anzeigenamen in dieser Community ändern.',
    channels: null,
  },
  {
    bit: Permission.MANAGE_NICKNAMES,
    label: 'Nicknames verwalten',
    description: 'Nicknames anderer Mitglieder ändern.',
    channels: null,
  },
  {
    bit: Permission.MANAGE_COMMUNITY,
    label: 'Community verwalten',
    description: 'Name, Aufbewahrung und Einladungen.',
    channels: null,
  },
  { bit: Permission.VIEW_AUDIT_LOG, label: 'Audit-Log einsehen', description: 'Sehen, wer was in der Community geändert hat.', channels: null },
  {
    bit: Permission.ADMINISTRATOR,
    label: 'Administrator',
    description: 'Alle Rechte, auch in eingeschränkten Kanälen. Mit Vorsicht vergeben.',
    channels: null,
  },
]

/** Gruppierung der Rechte im Rollen-Editor (wie bei Discord). */
export const PERMISSION_GROUPS: { title: string; bits: number[] }[] = [
  {
    title: 'Allgemeine Community-Rechte',
    bits: [
      Permission.VIEW_CHANNEL,
      Permission.MANAGE_CHANNELS,
      Permission.MANAGE_ROLES,
      Permission.MANAGE_COMMUNITY,
      Permission.MANAGE_WEBHOOKS,
      Permission.VIEW_AUDIT_LOG,
    ],
  },
  {
    title: 'Mitgliedschaft',
    bits: [Permission.CREATE_INVITES, Permission.CHANGE_NICKNAME, Permission.MANAGE_NICKNAMES, Permission.KICK_MEMBERS, Permission.BAN_MEMBERS],
  },
  {
    title: 'Textkanäle',
    bits: [
      Permission.SEND_MESSAGES,
      Permission.ATTACH_FILES,
      Permission.ADD_REACTIONS,
      Permission.MENTION_EVERYONE,
      Permission.MANAGE_MESSAGES,
    ],
  },
  {
    title: 'Sprachkanäle',
    bits: [Permission.CONNECT, Permission.SPEAK, Permission.VIDEO, Permission.STREAM],
  },
  { title: 'Erweitert', bits: [Permission.ADMINISTRATOR] },
]

/** Vorschläge für Rollenfarben (gut lesbar auf den dunklen Flächen). */
export const ROLE_COLOR_SUGGESTIONS = [
  '#1abc9c',
  '#2ecc71',
  '#3498db',
  '#9b59b6',
  '#e91e63',
  '#f1c40f',
  '#e67e22',
  '#e74c3c',
  '#95a5a6',
  '#607d8b',
  '#11806a',
  '#1f8b4c',
  '#206694',
  '#71368a',
  '#ad1457',
  '#c27c0e',
  '#a84300',
  '#992d22',
  '#979c9f',
  '#546e7a',
]

/** Enthält `have` alle Bits von `need`? (Ohne ADMINISTRATOR-Sonderregel – `have` ist bereits effektiv.) */
export function covers(have: number, need: number): boolean {
  return (need & ~have) === 0
}

/** Rollenfarbe als CSS-Wert. */
export function roleColor(color: number | null): string | undefined {
  return color == null ? undefined : `#${color.toString(16).padStart(6, '0')}`
}
