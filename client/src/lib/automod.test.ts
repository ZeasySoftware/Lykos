// AutoMod-Einstellungen und Alarm-Nachrichten. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  alertSummary,
  completeRules,
  defaultRule,
  formatList,
  invalidTerms,
  parseAlert,
  parseList,
  rulesEqual,
} from './automod.ts'

test('Listen aus Textfeldern: Zeilen und Kommas, ohne Leere und Dubletten', () => {
  assert.deepEqual(parseList(' spam \n\nSPAM, böse   wörter ,*foo*\n'), ['spam', 'böse wörter', '*foo*'])
  assert.deepEqual(parseList(''), [])
  assert.equal(formatList(['a', 'b']), 'a\nb')
})

test('ungültige Begriffe', () => {
  assert.deepEqual(invalidTerms(['ok', '*ok*', '*', 'f*o', '**x', 'x'.repeat(61)]), ['*', 'f*o', '**x', 'x'.repeat(61)])
  assert.deepEqual(invalidTerms(['klasse', 'kla*'], false), ['kla*'])
})

test('Regeln vervollständigen und vergleichen', () => {
  const rules = completeRules([{ ...defaultRule('spam'), enabled: true }])
  assert.deepEqual(
    rules.map((r) => r.kind),
    ['keyword', 'links', 'mentions', 'spam'],
  )
  assert.equal(rules[3].enabled, true)
  assert.equal(rules[0].enabled, false)

  const a = completeRules([{ ...defaultRule('keyword'), exempt_role_ids: ['r1', 'r2'] }])
  const b = completeRules([{ ...defaultRule('keyword'), exempt_role_ids: ['r2', 'r1'], custom_message: '  ' }])
  assert.ok(rulesEqual(a, b))
  const c = completeRules([{ ...defaultRule('keyword'), config: { terms: ['x'], allow: [] } }])
  assert.ok(!rulesEqual(a, c))
})

test('Alarm-Inhalt lesen', () => {
  const alert = parseAlert(
    JSON.stringify({
      rule: 'links',
      rule_name: 'Links blockieren',
      channel_id: 'c',
      blocked: true,
      edit: false,
      detail: 'Link zu x.de',
      excerpt: 'x.de',
    }),
  )
  assert.ok(alert)
  assert.equal(alert.detail, 'Link zu x.de')
  assert.equal(alertSummary(alert), 'Nachricht blockiert')
  assert.equal(alertSummary({ ...alert, blocked: false, edit: true }), 'Bearbeitung gemeldet')
  assert.equal(parseAlert('kein json'), null)
  assert.equal(parseAlert('null'), null)
  assert.equal(parseAlert('{"x":1}'), null)
})
