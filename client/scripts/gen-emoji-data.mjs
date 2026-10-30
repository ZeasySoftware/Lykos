// Erzeugt `src/data/emoji-data.json` (Unicode-Emojis für Picker, Suche und `:shortcode:`) aus `emojibase-data`.
//
// Einmalig ausführen, wenn eine neue Emoji-Version gewünscht ist:
//   npm i --no-save emojibase-data@18
//   node scripts/gen-emoji-data.mjs
//
// emojibase-data steht unter MIT-Lizenz, die Beschriftungen/Schlüsselwörter stammen aus Unicode CLDR
// (Unicode License). Die Ausgabe ist bewusst schlank (Arrays statt Objekte, Hauttöne meist nur als Flag) und wird
// per dynamischem Import erst geladen, wenn der Picker oder die Autovervollständigung sie braucht.
//
// Format: { v: 1, version, emojis: [emoji, shortcodes, name_de, keywords, group, skins?][] }
//   shortcodes: Leerzeichen-getrennt (iamcal/Slack-Stil wie bei Discord, ergänzt um GitHub-Aliasse)
//   keywords:   Leerzeichen-getrennt, kleingeschrieben (deutsche + englische Schlagwörter, englischer Name)
//   group:      Index in GROUPS (siehe src/lib/emojiData.ts)
//   skins:      1 = Hautton nach der Standardregel einsetzbar (Modifier nach dem ersten Codepunkt),
//               sonst die fünf Varianten als Array; fehlt = keine Hauttöne

import { mkdirSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const de = require('emojibase-data/de/data.json')
const en = require('emojibase-data/en/data.json')
const iamcal = require('emojibase-data/en/shortcodes/iamcal.json')
const github = require('emojibase-data/en/shortcodes/github.json')
const { version } = require('emojibase-data/package.json')

/** Höchste Emoji-Version: Windows 11 (Segoe UI Emoji) zeigt 15.0 vollständig an, neuere oft als Kästchen. */
const MAX_VERSION = 15.0

// emojibase-Gruppe → eigene Kategorie (Komponenten wie einzelne Hauttöne entfallen).
const GROUP_MAP = { 0: 0, 1: 0, 3: 1, 4: 2, 6: 3, 5: 4, 7: 5, 8: 6, 9: 7 }

const TONES = ['\u{1F3FB}', '\u{1F3FC}', '\u{1F3FD}', '\u{1F3FE}', '\u{1F3FF}']

/** Standardregel (identisch zu `applySkin` im Client): Modifier nach dem ersten Codepunkt, FE0F dahinter entfällt. */
function applyTone(emoji, tone) {
  const cps = Array.from(emoji)
  const rest = cps.slice(1)
  if (rest[0] === '️') rest.shift()
  return cps[0] + TONES[tone] + rest.join('')
}

const list = (v) => (v == null ? [] : Array.isArray(v) ? v : [v])
const enByHex = new Map(en.map((e) => [e.hexcode, e]))

const rows = []
for (const e of de) {
  if (e.group == null || !(e.group in GROUP_MAP) || e.version > MAX_VERSION) continue
  const english = enByHex.get(e.hexcode)
  const shortcodes = [...new Set([...list(iamcal[e.hexcode]), ...list(github[e.hexcode])])]
  if (shortcodes.length === 0) shortcodes.push(english.label.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, ''))
  const words = new Set()
  for (const w of [...(e.tags ?? []), ...(english?.tags ?? []), ...(english ? [english.label] : [])]) {
    for (const part of w.toLowerCase().split(/[\s:,]+/)) if (part.length > 1) words.add(part)
  }
  for (const part of e.label.toLowerCase().split(/[\s:,]+/)) words.delete(part)
  for (const s of shortcodes) words.delete(s)

  const row = [e.emoji, shortcodes.join(' '), e.label, [...words].join(' '), GROUP_MAP[e.group]]
  const skins = (e.skins ?? []).filter((s) => typeof s.tone === 'number' && s.version <= MAX_VERSION)
  if (skins.length === 5) {
    skins.sort((a, b) => a.tone - b.tone)
    const variants = skins.map((s) => s.emoji)
    row.push(variants.every((v, i) => v === applyTone(e.emoji, i)) ? 1 : variants)
  }
  rows.push({ order: e.order, row })
}
rows.sort((a, b) => a.order - b.order)

const out = join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'data', 'emoji-data.json')
mkdirSync(dirname(out), { recursive: true })
writeFileSync(out, JSON.stringify({ v: 1, version, emojis: rows.map((r) => r.row) }) + '\n')
console.log(`${rows.length} Emojis → ${out}`)
