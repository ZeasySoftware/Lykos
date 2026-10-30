// Beispiele für den Link „Issue auf GitHub anlegen“. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { publicIssueUrl } from './issues.ts'

test('GitHub-Repo → Auswahl der Issue-Vorlagen', () => {
  assert.equal(
    publicIssueUrl('https://github.com/ZeasySoftware/Lykos'),
    'https://github.com/ZeasySoftware/Lykos/issues/new/choose',
  )
  assert.equal(publicIssueUrl('https://github.com/a/b.git/'), 'https://github.com/a/b/issues/new/choose')
})

test('Andere Hoster, unvollständige oder ungültige Adressen → kein Link', () => {
  assert.equal(publicIssueUrl('https://git.example.org/a/b'), null)
  assert.equal(publicIssueUrl('http://github.com/a/b'), null)
  assert.equal(publicIssueUrl('https://github.com/a'), null)
  assert.equal(publicIssueUrl('kein link'), null)
  assert.equal(publicIssueUrl(undefined), null)
})
