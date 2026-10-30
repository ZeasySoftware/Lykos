// Rechte-Filter der Kontextmenü-Einträge. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Role } from './api.ts'
import { channelRules, communityRules, communityTabs, memberRules, messageRules, roleOptions, targetBelowMe } from './menuRules.ts'
import type { MemberRuleInput } from './menuRules.ts'
import { Permission } from './permissions.ts'

const P = Permission
const role = (id: string, position: number, permissions = 0, is_default = false): Role => ({
  id,
  community_id: 'c',
  name: id,
  permissions,
  position,
  color: null,
  is_default,
})
const roles = [role('everyone', 0, P.VIEW_CHANNEL, true), role('admin', 1, P.MANAGE_ROLES | P.KICK_MEMBERS), role('mod', 2, P.KICK_MEMBERS), role('member', 3)]

const base: MemberRuleInput = {
  myPerms: 0,
  iAmOwner: false,
  myRoleIds: ['everyone'],
  isSelf: false,
  targetIsOwner: false,
  targetRoleIds: ['everyone'],
  roles,
}

test('Nachrichten: Rechte bestimmen die Einträge', () => {
  const none = messageRules({ own: false, perms: P.VIEW_CHANNEL, isDm: false })
  assert.deepEqual(none, { react: false, reply: false, edit: false, pin: false, delete: false })
  const own = messageRules({ own: true, perms: P.SEND_MESSAGES | P.ADD_REACTIONS, isDm: false })
  assert.deepEqual(own, { react: true, reply: true, edit: true, pin: false, delete: true })
  const mod = messageRules({ own: false, perms: P.MANAGE_MESSAGES, isDm: false })
  assert.equal(mod.delete, true)
  assert.equal(mod.pin, true)
  // In DMs darf jeder anpinnen, aber keine fremden Nachrichten löschen.
  const dm = messageRules({ own: false, perms: P.SEND_MESSAGES | P.MANAGE_MESSAGES, isDm: true })
  assert.equal(dm.pin, true)
  assert.equal(dm.delete, false)
  // Administrator schließt alles ein.
  assert.equal(messageRules({ own: false, perms: P.ADMINISTRATOR, isDm: false }).delete, true)
})

test('Hierarchie: wer steht unter mir?', () => {
  assert.equal(targetBelowMe({ ...base, myRoleIds: ['admin'], targetRoleIds: ['mod'] }), true)
  assert.equal(targetBelowMe({ ...base, myRoleIds: ['mod'], targetRoleIds: ['admin'] }), false)
  assert.equal(targetBelowMe({ ...base, myRoleIds: ['mod'], targetRoleIds: ['mod'] }), false)
  // Nur Standardrolle steht unter jeder Rolle.
  assert.equal(targetBelowMe({ ...base, myRoleIds: ['member'], targetRoleIds: ['everyone'] }), true)
  // Ohne eigene Rolle steht niemand unter mir.
  assert.equal(targetBelowMe({ ...base, myRoleIds: ['everyone'], targetRoleIds: ['everyone'] }), false)
  // Besitzer steht über allen außer sich selbst.
  assert.equal(targetBelowMe({ ...base, iAmOwner: true, targetRoleIds: ['admin'] }), true)
  assert.equal(targetBelowMe({ ...base, iAmOwner: true, targetIsOwner: true }), false)
})

test('Mitglieder: Einträge ohne Recht werden ausgeblendet', () => {
  assert.deepEqual(memberRules(base), { message: true, nickname: false, roles: false, kick: false })
  const self = memberRules({ ...base, isSelf: true, myPerms: P.CHANGE_NICKNAME })
  assert.equal(self.message, false)
  assert.equal(self.nickname, true)
  assert.equal(self.kick, false)

  const mod = { ...base, myPerms: P.KICK_MEMBERS | P.MANAGE_NICKNAMES | P.MANAGE_ROLES, myRoleIds: ['mod'] }
  assert.deepEqual(memberRules({ ...mod, targetRoleIds: ['member'] }), { message: true, nickname: true, roles: true, kick: true })
  // Höherrangiges Mitglied: Nickname nicht, Entfernen schon (Server prüft beim Kick keine Hierarchie).
  assert.equal(memberRules({ ...mod, targetRoleIds: ['admin'] }).nickname, false)
  // Besitzer kann niemand entfernen oder umbenennen.
  const owner = memberRules({ ...mod, targetIsOwner: true })
  assert.equal(owner.kick, false)
  assert.equal(owner.nickname, false)
  // Kein Mitglied (z. B. DM-Partner): nur Nachricht.
  assert.deepEqual(memberRules({ ...mod, targetRoleIds: undefined }), { message: true, nickname: false, roles: false, kick: false })
})

test('Rollen-Untermenü: Hierarchie und eigene Rechte', () => {
  const opts = roleOptions({
    ...base,
    myPerms: P.MANAGE_ROLES | P.KICK_MEMBERS,
    myRoleIds: ['admin'],
    targetRoleIds: ['everyone', 'member'],
  })
  assert.deepEqual(
    opts.map((o) => [o.role.id, o.checked, o.enabled]),
    [
      ['admin', false, false],
      ['mod', false, true],
      ['member', true, true],
    ],
  )
  assert.match(opts[0].reason ?? '', /höchsten Rolle/)
  // Rolle mit Rechten, die man nicht hat.
  const weak = roleOptions({ ...base, myPerms: P.MANAGE_ROLES, iAmOwner: false, myRoleIds: ['admin'] })
  const mod = weak.find((o) => o.role.id === 'mod')!
  assert.equal(mod.enabled, false)
  assert.match(mod.reason ?? '', /Rechte/)
})

test('Kanäle und Communities', () => {
  assert.deepEqual(channelRules({ channelPerms: P.VIEW_CHANNEL, communityPerms: P.CREATE_INVITES, kind: 'text' }), {
    markRead: true,
    invite: true,
    edit: false,
    delete: false,
    connect: false,
  })
  const voice = channelRules({ channelPerms: P.CONNECT | P.MANAGE_ROLES, communityPerms: 0, kind: 'voice' })
  assert.equal(voice.markRead, false)
  assert.equal(voice.connect, true)
  assert.equal(voice.edit, true)
  assert.equal(voice.delete, false)

  assert.deepEqual(communityRules(0, false), { invite: false, createChannel: false, leave: true, delete: false })
  assert.deepEqual(communityRules(P.ADMINISTRATOR, true), { invite: true, createChannel: true, leave: false, delete: true })
  assert.deepEqual(
    communityTabs(P.KICK_MEMBERS, false).map((t) => t.id),
    ['overview', 'members', 'danger'],
  )
  assert.equal(communityTabs(P.ADMINISTRATOR, true).length, 8)
})
