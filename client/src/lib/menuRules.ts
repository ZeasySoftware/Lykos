/**
 * Rechte-Regeln für Kontextmenüs und Einstellungen – rein, ohne React/Store, damit testbar.
 * Sie spiegeln die Server-Prüfungen (siehe docs/api.md); Einträge ohne Recht werden ausgeblendet.
 */
import type { Role } from './api'
import { Permission, has } from './permissions.ts'
import { outranks, topPosition } from './roles.ts'

/** Enthält `have` alle Bits von `need`? (`have` ist bereits effektiv.) */
function covers(have: number, need: number): boolean {
  return (need & ~have) === 0
}

// ---------------------------------------------------------------------------
// Nachrichten
// ---------------------------------------------------------------------------

export interface MessageRuleInput {
  /** Eigene Nachricht? */
  own: boolean
  /** Effektive Rechte im Kanal. */
  perms: number
  /** In DMs dürfen alle Teilnehmer anpinnen. */
  isDm: boolean
}

export function messageRules({ own, perms, isDm }: MessageRuleInput) {
  const manage = has(perms, Permission.MANAGE_MESSAGES)
  return {
    react: has(perms, Permission.ADD_REACTIONS),
    reply: has(perms, Permission.SEND_MESSAGES),
    edit: own,
    pin: manage || isDm,
    delete: own || (manage && !isDm),
  }
}

// ---------------------------------------------------------------------------
// Mitglieder (Nutzer-Kontextmenü)
// ---------------------------------------------------------------------------

export interface MemberRuleInput {
  /** Eigene effektive Rechte auf Community-Ebene. */
  myPerms: number
  iAmOwner: boolean
  /** Eigene Rollen (`undefined`, solange die Mitgliederliste fehlt). */
  myRoleIds: string[] | undefined
  /** Ziel ist man selbst? */
  isSelf: boolean
  /** Ziel ist der Besitzer der Community? */
  targetIsOwner: boolean
  /** Rollen des Ziels (`undefined`, wenn kein Mitglied/unbekannt). */
  targetRoleIds: string[] | undefined
  roles: Role[]
}

/**
 * Steht das Ziel in der Hierarchie echt unter mir? Mitglieder nur mit Standardrolle stehen unter jeder
 * Rolle; ohne eigene Rolle steht niemand unter mir (Besitzer ausgenommen).
 */
export function targetBelowMe(input: MemberRuleInput): boolean {
  if (input.iAmOwner) return !input.targetIsOwner
  const mine = topPosition(input.roles, input.myRoleIds)
  if (mine == null) return false
  const theirs = topPosition(input.roles, input.targetRoleIds ?? [])
  return theirs === null || (theirs !== undefined && theirs > mine)
}

export function memberRules(input: MemberRuleInput) {
  const { myPerms, isSelf, targetIsOwner, targetRoleIds } = input
  const isMember = targetRoleIds !== undefined
  const nickname = !isMember
    ? false
    : isSelf
      ? has(myPerms, Permission.CHANGE_NICKNAME) || has(myPerms, Permission.MANAGE_NICKNAMES)
      : has(myPerms, Permission.MANAGE_NICKNAMES) && !targetIsOwner && targetBelowMe(input)
  return {
    /** Direktnachricht nur an andere. */
    message: !isSelf,
    nickname,
    roles: isMember && has(myPerms, Permission.MANAGE_ROLES),
    kick: isMember && !isSelf && !targetIsOwner && has(myPerms, Permission.KICK_MEMBERS),
    /** Bannen folgt (wie serverseitig) der Rollenhierarchie. */
    ban: isMember && !isSelf && !targetIsOwner && has(myPerms, Permission.BAN_MEMBERS) && targetBelowMe(input),
  }
}

export interface RoleOption {
  role: Role
  checked: boolean
  /** Darf ich die Rolle zuweisen/entziehen (Hierarchie + eigene Rechte)? */
  enabled: boolean
  /** Grund, falls nicht. */
  reason?: string
}

/** Zuweisbare Rollen (ohne Standardrolle), höchste zuerst, mit Haken und Sperrgrund. */
export function roleOptions(input: MemberRuleInput): RoleOption[] {
  const top = topPosition(input.roles, input.myRoleIds)
  return input.roles
    .filter((r) => !r.is_default)
    .sort((a, b) => a.position - b.position || (a.id < b.id ? -1 : 1))
    .map((role) => {
      const ranked = outranks(role, input.iAmOwner, top)
      const owned = covers(input.myPerms, role.permissions)
      return {
        role,
        checked: !!input.targetRoleIds?.includes(role.id),
        enabled: ranked && owned,
        reason: !ranked
          ? 'Diese Rolle steht nicht unter deiner höchsten Rolle.'
          : !owned
            ? 'Diese Rolle hat Rechte, die du nicht besitzt.'
            : undefined,
      }
    })
}

// ---------------------------------------------------------------------------
// Kanäle & Communities
// ---------------------------------------------------------------------------

export function channelRules(input: { channelPerms: number; communityPerms: number; kind: 'text' | 'voice' }) {
  const { channelPerms, communityPerms, kind } = input
  return {
    markRead: kind === 'text',
    invite: has(communityPerms, Permission.CREATE_INVITES),
    edit:
      has(channelPerms, Permission.MANAGE_CHANNELS) ||
      has(channelPerms, Permission.MANAGE_ROLES) ||
      (kind === 'text' && has(channelPerms, Permission.MANAGE_WEBHOOKS)),
    delete: has(channelPerms, Permission.MANAGE_CHANNELS),
    connect: kind === 'voice' && has(channelPerms, Permission.CONNECT),
  }
}

export type CommunityTabId =
  | 'overview'
  | 'appearance'
  | 'emojis'
  | 'roles'
  | 'members'
  | 'bans'
  | 'invites'
  | 'automod'
  | 'audit'
  | 'danger'

/** Bereiche der Community-Einstellungen, die der Nutzer sehen darf. */
export function communityTabs(perms: number, isOwner: boolean): { id: CommunityTabId; label: string }[] {
  const tabs: { id: CommunityTabId; label: string; visible: boolean }[] = [
    { id: 'overview', label: 'Übersicht', visible: true },
    { id: 'appearance', label: 'Darstellung', visible: has(perms, Permission.MANAGE_COMMUNITY) },
    { id: 'roles', label: 'Rollen', visible: has(perms, Permission.MANAGE_ROLES) },
    {
      id: 'members',
      label: 'Mitglieder',
      visible:
        has(perms, Permission.MANAGE_ROLES) ||
        has(perms, Permission.KICK_MEMBERS) ||
        has(perms, Permission.BAN_MEMBERS) ||
        has(perms, Permission.MANAGE_NICKNAMES),
    },
    { id: 'bans', label: 'Banns', visible: has(perms, Permission.BAN_MEMBERS) },
    { id: 'invites', label: 'Einladungen', visible: has(perms, Permission.MANAGE_COMMUNITY) },
    { id: 'emojis', label: 'Emojis', visible: has(perms, Permission.MANAGE_EMOJIS) },
    { id: 'automod', label: 'AutoMod', visible: has(perms, Permission.MANAGE_COMMUNITY) },
    { id: 'audit', label: 'Audit-Log', visible: has(perms, Permission.VIEW_AUDIT_LOG) },
    { id: 'danger', label: isOwner ? 'Community löschen' : 'Community verlassen', visible: true },
  ]
  return tabs.filter((t) => t.visible).map(({ id, label }) => ({ id, label }))
}

export function communityRules(perms: number, isOwner: boolean) {
  return {
    invite: has(perms, Permission.CREATE_INVITES),
    createChannel: has(perms, Permission.MANAGE_CHANNELS),
    leave: !isOwner,
    delete: isOwner,
  }
}
