// AutoMod: reine Logik für die Einstellungsseite und die Alarm-Systemnachricht (ohne React).
import type { AutoModAlertData, AutoModRule, AutoModRuleKind } from './api.ts'

/** Obergrenzen wie im Server (`automod.rs`, `automod/filters.rs`). */
export const AUTOMOD_LIMITS = {
  terms: 1000,
  termChars: 60,
  domains: 200,
  customMessage: 150,
} as const

/** Reihenfolge und Texte der Regelkarten. */
export const AUTOMOD_RULES: { kind: AutoModRuleKind; title: string; description: string; blockMessage: string }[] = [
  {
    kind: 'keyword',
    title: 'Wortfilter',
    description: 'Blockiert Nachrichten mit bestimmten Begriffen – Groß-/Kleinschreibung egal.',
    blockMessage: 'Deine Nachricht enthält einen Begriff, der in dieser Community nicht erlaubt ist.',
  },
  {
    kind: 'links',
    title: 'Links blockieren',
    description: 'Erlaubt Links nur zu freigegebenen Domains. Links zu dieser Lykos-Instanz gehen immer.',
    blockMessage: 'Links sind in dieser Community nur zu freigegebenen Seiten erlaubt.',
  },
  {
    kind: 'mentions',
    title: 'Erwähnungs-Spam',
    description: 'Greift, wenn eine Nachricht zu viele Mitglieder auf einmal erwähnt.',
    blockMessage: 'Deine Nachricht erwähnt zu viele Personen auf einmal.',
  },
  {
    kind: 'spam',
    title: 'Spam & Flut',
    description: 'Greift bei zu vielen Nachrichten in kurzer Zeit oder immer gleichem Text.',
    blockMessage: 'Du schreibst zu schnell oder wiederholst dich. Warte einen Moment.',
  },
]

/** Voreinstellung einer Regel (wie der Server: aus, blockieren). */
export function defaultRule(kind: AutoModRuleKind): AutoModRule {
  const base = {
    enabled: false,
    actions: { block: true, alert: false, audit: false },
    alert_channel_id: null,
    custom_message: null,
    exempt_role_ids: [],
    exempt_channel_ids: [],
  }
  switch (kind) {
    case 'keyword':
      return { ...base, kind, config: { terms: [], allow: [] } }
    case 'links':
      return { ...base, kind, config: { allowed_domains: [], block_foreign_invites: true } }
    case 'mentions':
      return { ...base, kind, config: { max_mentions: 5 } }
    case 'spam':
      return { ...base, kind, config: { max_messages: 5, per_seconds: 5, max_duplicates: 3 } }
  }
}

/** Alle vier Regeln in fester Reihenfolge; fehlende bekommen Voreinstellungen. */
export function completeRules(rules: AutoModRule[]): AutoModRule[] {
  return AUTOMOD_RULES.map(({ kind }) => rules.find((r) => r.kind === kind) ?? defaultRule(kind))
}

/** Liste aus einem Textfeld: eine Zeile oder ein Komma je Eintrag, getrimmt, ohne Leere und Dubletten (Groß/Klein egal). */
export function parseList(text: string): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const raw of text.split(/[\n,]/)) {
    const item = raw.trim().replace(/\s+/g, ' ')
    const key = item.toLocaleLowerCase('de')
    if (item && !seen.has(key)) {
      seen.add(key)
      out.push(item)
    }
  }
  return out
}

/** Liste für ein Textfeld (eine Zeile je Eintrag). */
export function formatList(items: string[]): string {
  return items.join('\n')
}

/** Ungültige Begriffe (zu lang, nur `*`, `*` mitten im Begriff, mehrfach am Rand) – für den Hinweis unter dem Feld. */
export function invalidTerms(terms: string[], allowWildcards = true): string[] {
  return terms.filter((t) => {
    if ([...t].length > AUTOMOD_LIMITS.termChars) return true
    const core = t.replace(/^\*/, '').replace(/\*$/, '')
    if (!core.trim() || core.includes('*')) return true
    return !allowWildcards && t.includes('*')
  })
}

/** Regel für den Vergleich „ungespeicherte Änderungen“ (Reihenfolge von Ausnahmen egal). */
function canonical(rule: AutoModRule): string {
  return JSON.stringify({
    ...rule,
    custom_message: rule.custom_message?.trim() || null,
    exempt_role_ids: [...rule.exempt_role_ids].sort(),
    exempt_channel_ids: [...rule.exempt_channel_ids].sort(),
  })
}

export function rulesEqual(a: AutoModRule[], b: AutoModRule[]): boolean {
  return a.length === b.length && a.every((rule, i) => canonical(rule) === canonical(b[i]))
}

/** Inhalt einer `automod_alert`-Nachricht lesen; `null` bei unlesbarem Inhalt. */
export function parseAlert(content: string): AutoModAlertData | null {
  try {
    const data = JSON.parse(content) as Partial<AutoModAlertData> | null
    if (!data || typeof data !== 'object' || typeof data.rule !== 'string') return null
    return {
      rule: data.rule,
      rule_name: typeof data.rule_name === 'string' ? data.rule_name : data.rule,
      channel_id: typeof data.channel_id === 'string' ? data.channel_id : '',
      blocked: data.blocked === true,
      edit: data.edit === true,
      detail: typeof data.detail === 'string' ? data.detail : '',
      excerpt: typeof data.excerpt === 'string' ? data.excerpt : '',
    }
  } catch {
    return null
  }
}

/** Kurzfassung eines Alarms: „Nachricht blockiert“, „Bearbeitung gemeldet“ … */
export function alertSummary(alert: AutoModAlertData): string {
  const what = alert.edit ? 'Bearbeitung' : 'Nachricht'
  return alert.blocked ? `${what} blockiert` : `${what} gemeldet`
}
