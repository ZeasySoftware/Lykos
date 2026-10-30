// Reine Logik der Desktop-Hülle. Ausführen mit `npm test` (Node mit Type-Stripping).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  closeChoicePatch,
  closeMode,
  closeNeedsCallWarning,
  initialCloseDecided,
  isWindows11,
  titlebarContext,
  trayBadge,
  trayTooltip,
} from './desktopShell.ts'

test('Titelleiste: Community, Kanal und DMs', () => {
  assert.equal(titlebarContext({ dmView: false }), null)
  assert.equal(titlebarContext({ dmView: false, communityName: 'Rudel' }), 'Rudel')
  assert.equal(
    titlebarContext({ dmView: false, communityName: 'Rudel', channel: { name: 'allgemein', kind: 'text' } }),
    'Rudel · #allgemein',
  )
  assert.equal(
    titlebarContext({ dmView: false, communityName: 'Rudel', channel: { name: 'Lounge', kind: 'voice' } }),
    'Rudel · Lounge',
  )
  assert.equal(titlebarContext({ dmView: true }), 'Direktnachrichten')
  assert.equal(titlebarContext({ dmView: true, dmName: 'Anna', communityName: 'Rudel' }), 'Direktnachrichten · Anna')
})

test('Tray-Punkt: Erwähnungen und DMs vor sonstigem Ungelesenen', () => {
  const none = { mentions: 0, dmUnread: false, communityUnread: false }
  assert.equal(trayBadge(none), 'none')
  assert.equal(trayBadge({ ...none, communityUnread: true }), 'unread')
  assert.equal(trayBadge({ ...none, communityUnread: true, mentions: 2 }), 'mention')
  assert.equal(trayBadge({ ...none, dmUnread: true }), 'mention')
})

test('Tray-Tooltip', () => {
  const none = { mentions: 0, dmUnread: false, communityUnread: false }
  assert.equal(trayTooltip(none, false), 'Lykos')
  assert.equal(trayTooltip({ ...none, mentions: 1 }, false), 'Lykos – 1 ungelesene Erwähnung')
  assert.equal(trayTooltip({ ...none, mentions: 3, dmUnread: true }, true), 'Lykos – 3 ungelesene Erwähnungen – Im Sprachkanal')
  assert.equal(trayTooltip({ ...none, dmUnread: true }, false), 'Lykos – Ungelesene Direktnachrichten')
  assert.equal(trayTooltip({ ...none, communityUnread: true }, true), 'Lykos – Ungelesene Nachrichten – Im Sprachkanal')
})

test('Windows 11 erkennen', () => {
  assert.equal(isWindows11('Windows', '15.0.0'), true)
  assert.equal(isWindows11('Windows', '13.0.0'), true)
  assert.equal(isWindows11('Windows', '10.0.0'), false)
  assert.equal(isWindows11('Windows', '0.3.0'), false)
  assert.equal(isWindows11('Windows', undefined), false)
  assert.equal(isWindows11('macOS', '15.0.0'), false)
})

test('Schließen: Modus aus den Einstellungen', () => {
  assert.equal(closeMode(true, false), 'ask')
  assert.equal(closeMode(false, false), 'ask')
  assert.equal(closeMode(true, true), 'tray')
  assert.equal(closeMode(false, true), 'quit')
})

test('Schließen: Bestandsnutzer mit geänderter Einstellung gelten als entschieden', () => {
  assert.equal(initialCloseDecided({}), false)
  assert.equal(initialCloseDecided({ closeToTray: true }), false)
  assert.equal(initialCloseDecided({ closeToTray: false }), true)
  assert.equal(initialCloseDecided({ closeToTray: true, closeDecided: true }), true)
  assert.equal(initialCloseDecided({ closeToTray: false, closeDecided: false }), false)
})

test('Schließen: Antwort im Dialog merken oder nur dieses Mal', () => {
  assert.deepEqual(closeChoicePatch('tray', true), { closeToTray: true, closeDecided: true })
  assert.deepEqual(closeChoicePatch('quit', true), { closeToTray: false, closeDecided: true })
  assert.equal(closeChoicePatch('quit', false), null)
  assert.equal(closeNeedsCallWarning('quit', true), true)
  assert.equal(closeNeedsCallWarning('quit', false), false)
  assert.equal(closeNeedsCallWarning('tray', true), false)
})
