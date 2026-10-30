// Vorlagen-Code aus Discord-Links. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { extractTemplateCode } from './discordTemplate.ts'

test('Code aus Links und nacktem Code', () => {
  for (const input of [
    'HqjFNRCT6EEU',
    '  HqjFNRCT6EEU ',
    'https://discord.new/HqjFNRCT6EEU',
    'discord.new/HqjFNRCT6EEU',
    'https://discord.com/template/HqjFNRCT6EEU',
    'https://www.discord.com/template/HqjFNRCT6EEU?x=1#y',
    'https://discordapp.com/template/HqjFNRCT6EEU',
    'https://discord.gg/template/HqjFNRCT6EEU',
  ]) {
    assert.equal(extractTemplateCode(input), 'HqjFNRCT6EEU', input)
  }
})

test('fremde Hosts und Unsinn werden abgelehnt', () => {
  for (const input of [
    '',
    'a',
    'https://evil.example/template/HqjFNRCT6EEU',
    'https://discord.new.evil.example/HqjFNRCT6EEU',
    'https://discord.new@evil.example/HqjFNRCT6EEU',
    'https://discord.new:8443/HqjFNRCT6EEU',
    'https://discord.com/invite/HqjFNRCT6EEU',
    'https://discord.new/a/b',
    'ftp://discord.new/HqjFNRCT6EEU',
    'Hqj FNRCT',
  ]) {
    assert.equal(extractTemplateCode(input), null, input)
  }
})
