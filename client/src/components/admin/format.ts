/** Formatierung für die Instanz-Verwaltung. */

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })
const dateTimeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })
const numberFmt = new Intl.NumberFormat('de-DE')

export function fmtDate(iso: string | null | undefined): string {
  return iso ? dateFmt.format(new Date(iso)) : '–'
}

export function fmtDateTime(iso: string | null | undefined): string {
  return iso ? dateTimeFmt.format(new Date(iso)) : '–'
}

/** Tag im Format `YYYY-MM-DD` (ohne Zeitzonen-Verschiebung) als deutsches Datum. */
export function fmtDay(day: string | null | undefined): string {
  if (!day) return '–'
  const [y, m, d] = day.split('-').map(Number)
  return dateFmt.format(new Date(y, m - 1, d))
}

export function fmtNumber(n: number): string {
  return numberFmt.format(n)
}

/** Anzahl mit Singular/Plural: `fmtCount(1, 'Datei', 'Dateien')` → „1 Datei“. */
export function fmtCount(n: number, one: string, many: string): string {
  return `${numberFmt.format(n)} ${n === 1 ? one : many}`
}

/** Laufzeit grob: „3 T 4 Std“, „5 Std 12 Min“, „42 Min“. */
export function fmtUptime(seconds: number): string {
  const days = Math.floor(seconds / 86_400)
  const hours = Math.floor((seconds % 86_400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (days > 0) return `${days} T ${hours} Std`
  if (hours > 0) return `${hours} Std ${minutes} Min`
  return `${minutes} Min`
}

/** Anzeigename eines Nutzers (Anzeigename, sonst Benutzername). */
export function nameOf(u: { username: string; display_name: string | null } | null | undefined): string {
  if (!u) return 'Gelöschter Nutzer'
  return u.display_name || u.username
}
