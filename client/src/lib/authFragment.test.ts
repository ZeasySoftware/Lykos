import { test } from 'node:test'
import assert from 'node:assert/strict'
import { linkToken, parseAuthFragment } from './authFragment.ts'

test('OIDC-Fragmente werden erkannt', () => {
  assert.deepEqual(parseAuthFragment('#oidc_signup=abc'), { kind: 'signup', token: 'abc' })
  assert.deepEqual(parseAuthFragment('#mfa_token=t1&mfa_methods=totp,recovery,quatsch'), {
    kind: 'mfa',
    token: 't1',
    methods: ['totp', 'recovery'],
  })
  assert.deepEqual(parseAuthFragment('#mfa_token=t1'), { kind: 'mfa', token: 't1', methods: ['recovery'] })
  assert.deepEqual(parseAuthFragment('#oidc_linked=keycloak'), { kind: 'linked', provider: 'keycloak' })
  assert.deepEqual(parseAuthFragment('#oidc_error=cancelled'), { kind: 'error', code: 'cancelled' })
  assert.equal(parseAuthFragment(''), null)
  assert.equal(parseAuthFragment('#irgendwas'), null)
})

test('Link-Token aus dem Fragment, übergangsweise auch aus der Query', () => {
  assert.equal(linkToken('#token=abc', ''), 'abc')
  assert.equal(linkToken('', '?token=alt'), 'alt')
  assert.equal(linkToken('#token=neu', '?token=alt'), 'neu')
  assert.equal(linkToken('#oidc_error=x', ''), null)
  assert.equal(linkToken('', ''), null)
})
