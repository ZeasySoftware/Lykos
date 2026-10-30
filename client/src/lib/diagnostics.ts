/**
 * Technische Infos für Fehlermeldungen (Paket K).
 *
 * Ein kleiner Ringpuffer im Arbeitsspeicher merkt sich die letzten Client-Fehler (`console.error`,
 * `window.onerror`, `unhandledrejection`). Er verlässt das Gerät nur, wenn der Nutzer beim Melden
 * „Technische Infos anhängen“ angehakt lässt – nichts wird gespeichert oder automatisch gesendet.
 *
 * Datenschutz: Es werden nur Fehlertexte übernommen, keine Objekte (also keine Nachrichten, Nutzer,
 * Antworten). Alles wird gekürzt und geschwärzt: IDs, Tokens, E-Mail-Adressen, Query-Strings.
 *
 * Bewusst ohne Abhängigkeiten (kein `api.ts`), damit `node --test` das Modul direkt laden kann.
 */

/** So viele Fehler hält der Puffer höchstens. */
export const MAX_ERRORS = 50
/** Höchstlänge eines Eintrags (Zeichen). */
export const MAX_MESSAGE_CHARS = 300

export interface ClientError {
  /** Zeitpunkt (ISO 8601). */
  at: string
  message: string
}

export interface TechInfo {
  app_version: string | null
  platform: 'web' | 'desktop'
  os: string | null
  window: { width: number; height: number } | null
  view: string | null
  errors: ClientError[]
}

/** Fester Ringpuffer: `push` verdrängt den ältesten Eintrag. */
export class RingBuffer<T> {
  private items: T[] = []
  private readonly capacity: number

  constructor(capacity: number) {
    this.capacity = Math.max(1, capacity)
  }

  push(item: T) {
    this.items.push(item)
    if (this.items.length > this.capacity) this.items.splice(0, this.items.length - this.capacity)
  }

  /** Inhalt, älteste zuerst (Kopie). */
  toArray(): T[] {
    return [...this.items]
  }

  clear() {
    this.items = []
  }

  get size(): number {
    return this.items.length
  }
}

const UUID = /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi
const EMAIL = /[^\s@<>()"']+@[^\s@<>()"']+\.[a-z]{2,}/gi
const BEARER = /\b(bearer|token)(\s*[:=]?\s*)\S+/gi
const URL_QUERY = /(https?:\/\/[^\s?#"']+)[?#][^\s"']*/gi
/** Lange Zeichenketten mit mindestens einer Ziffer: Tokens, Hashes, Schlüssel. */
const LONG_TOKEN = /\b(?=[A-Za-z0-9_-]*\d)[A-Za-z0-9_-]{24,}\b/g

function isControl(c: string): boolean {
  const code = c.codePointAt(0) ?? 0
  return code < 0x20 || code === 0x7f
}

/** Schwärzt IDs, Tokens, E-Mail-Adressen und Query-Strings; kürzt auf `max` Zeichen. */
export function redact(text: string, max = MAX_MESSAGE_CHARS): string {
  const cleaned = text
    .replace(URL_QUERY, '$1?…')
    .replace(EMAIL, '<email>')
    .replace(BEARER, '$1$2<redacted>')
    .replace(UUID, '<id>')
    .replace(LONG_TOKEN, '<redacted>')
  // Steuerzeichen außer Zeilenumbruch durch Leerzeichen ersetzen.
  const chars = Array.from(cleaned, (c) => (c !== '\n' && isControl(c) ? ' ' : c))
  const visible = chars.join('').trim()
  const visibleChars = Array.from(visible)
  return visibleChars.length > max ? `${visibleChars.slice(0, max - 1).join('')}…` : visible
}

/**
 * Einen Wert aus `console.error`/`reject` als Text darstellen – ohne Objektinhalte: Fehler werden zu
 * „Name: Meldung“, Zeichenketten bleiben, alles andere wird nur als Typ vermerkt.
 */
export function describe(value: unknown): string {
  if (value instanceof Error) return `${value.name}: ${value.message}`
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean' || value === null || value === undefined) {
    return String(value)
  }
  if (typeof value === 'object') {
    // Fehlerähnliche Objekte (z. B. ApiError aus anderen Realms, DOMException).
    const v = value as { name?: unknown; message?: unknown; code?: unknown }
    if (typeof v.message === 'string') return `${typeof v.name === 'string' ? v.name : 'Fehler'}: ${v.message}`
    if (typeof v.code === 'string') return `[Code ${v.code}]`
    return '[Objekt]'
  }
  return `[${typeof value}]`
}

/** Betriebssystem grob aus dem User-Agent (ohne Version). */
export function coarseOs(userAgent: string): string | null {
  const ua = userAgent.toLowerCase()
  if (ua.includes('windows')) return 'Windows'
  if (ua.includes('android')) return 'Android'
  if (/iphone|ipad|ipod/.test(ua)) return 'iOS'
  if (ua.includes('mac os') || ua.includes('macintosh')) return 'macOS'
  if (ua.includes('cros')) return 'ChromeOS'
  if (ua.includes('linux')) return 'Linux'
  return null
}

/** Pfad ohne IDs und Codes, z. B. `/invite/abc123` → `/invite/<code>`. */
export function viewFromPath(pathname: string): string {
  const parts = pathname.split('/').filter(Boolean)
  if (parts[0] === 'invite' && parts.length > 1) return '/invite/<code>'
  return '/' + parts.map((p) => redact(p, 40)).join('/')
}

// ---------------------------------------------------------------------------
// Erfassen
// ---------------------------------------------------------------------------

const buffer = new RingBuffer<ClientError>(MAX_ERRORS)
let installed = false

/** Fehler in den Puffer legen (bereits geschwärzt). */
export function recordError(parts: unknown[], now = new Date()) {
  const message = redact(parts.map(describe).join(' '))
  if (message) buffer.push({ at: now.toISOString(), message })
}

/** Aktuell gepufferte Fehler, älteste zuerst. */
export function recentErrors(): ClientError[] {
  return buffer.toArray()
}

/** Puffer leeren (Tests, Abmelden). */
export function clearErrors() {
  buffer.clear()
}

interface ErrorTarget {
  addEventListener(type: 'error', cb: (e: { message?: string; filename?: string; lineno?: number; error?: unknown }) => void): void
  addEventListener(type: 'unhandledrejection', cb: (e: { reason?: unknown }) => void): void
}

/**
 * `console.error`, `window.onerror` und `unhandledrejection` mitschneiden. Die ursprüngliche
 * Konsolenausgabe bleibt unverändert. Mehrfacher Aufruf ist harmlos.
 */
export function installDiagnostics(target: ErrorTarget = window as unknown as ErrorTarget, con: Console = console) {
  if (installed) return
  installed = true

  const original = con.error.bind(con)
  con.error = (...args: unknown[]) => {
    try {
      recordError(args)
    } catch {
      // Diagnose darf nie selbst stören.
    }
    original(...args)
  }
  target.addEventListener('error', (e) => {
    const file = e.filename ? e.filename.split(/[\\/]/).pop()?.split('?')[0] : undefined
    const where = file ? `(${file}:${e.lineno ?? 0})` : ''
    recordError([e.error instanceof Error ? e.error : (e.message ?? 'Unbekannter Fehler'), where].filter(Boolean))
  })
  target.addEventListener('unhandledrejection', (e) => {
    recordError(['Unbehandelte Ablehnung:', e.reason])
  })
}

/** Umgebung, aus der `collectTechInfo` liest (im Browser `window`/`navigator`, in Tests frei wählbar). */
export interface TechEnvironment {
  appVersion: string
  desktop: boolean
  userAgent: string
  width: number
  height: number
  /** Pfad (wird ohne IDs übernommen). */
  pathname: string
  /** Beschreibung der Ansicht aus dem App-Zustand, z. B. `community/textkanal`. */
  area: string | null
}

/** Technische Infos zusammenstellen – genau das, was beim Absenden mitgeht. */
export function collectTechInfo(env: TechEnvironment, errors: ClientError[] = recentErrors()): TechInfo {
  const path = viewFromPath(env.pathname)
  const view = [path === '/' ? null : path, env.area].filter(Boolean).join(' · ') || '/'
  return {
    app_version: env.appVersion || null,
    platform: env.desktop ? 'desktop' : 'web',
    os: coarseOs(env.userAgent),
    window: env.width > 0 && env.height > 0 ? { width: Math.round(env.width), height: Math.round(env.height) } : null,
    view: redact(view, 64),
    errors: errors.slice(-MAX_ERRORS),
  }
}
