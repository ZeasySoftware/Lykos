import type { MfaMethod } from './api'

/**
 * Ergebnis der Rückkehr vom OIDC-Anbieter. Der Server leitet mit dem Ergebnis im URL-Fragment zurück
 * (`/#oidc_signup=…`, `/#mfa_token=…&mfa_methods=…`, `/#oidc_linked=…`, `/#oidc_error=…`) – das Fragment
 * geht nie an einen Server. Es wird beim Laden einmal gelesen und sofort aus der Adresszeile entfernt.
 */
export type AuthFragment =
  | { kind: 'signup'; token: string }
  | { kind: 'mfa'; token: string; methods: MfaMethod[] }
  | { kind: 'linked'; provider: string }
  | { kind: 'error'; code: string }

const METHODS: MfaMethod[] = ['totp', 'email', 'recovery']

export function parseAuthFragment(hash: string): AuthFragment | null {
  const params = new URLSearchParams(hash.replace(/^#/, ''))
  const signup = params.get('oidc_signup')
  if (signup) return { kind: 'signup', token: signup }
  const mfa = params.get('mfa_token')
  if (mfa) {
    const methods = (params.get('mfa_methods') ?? '')
      .split(',')
      .filter((m): m is MfaMethod => (METHODS as string[]).includes(m))
    return { kind: 'mfa', token: mfa, methods: methods.length ? methods : ['recovery'] }
  }
  const linked = params.get('oidc_linked')
  if (linked) return { kind: 'linked', provider: linked }
  const error = params.get('oidc_error')
  if (error) return { kind: 'error', code: error }
  return null
}

/**
 * Einmal-Token aus Bestätigungs- bzw. Reset-Links (`/verify-email#token=…`, `/reset-password#token=…`). Im
 * Fragment geht das Token weder an den Server noch per `Referer` weiter; ältere Mails enthalten noch
 * `?token=…` – das wird übergangsweise weiter akzeptiert. `null`, wenn keines da ist.
 */
export function linkToken(hash: string, search: string): string | null {
  return new URLSearchParams(hash.replace(/^#/, '')).get('token') || new URLSearchParams(search).get('token')
}

let pending: AuthFragment | null = null

if (typeof location !== 'undefined') {
  pending = parseAuthFragment(location.hash)
  if (pending) history.replaceState(null, '', location.pathname + location.search)
}

/** Fragment-Ergebnis abholen (nur einmal; danach `null`). */
export function takeAuthFragment(kinds: AuthFragment['kind'][]): AuthFragment | null {
  if (!pending || !kinds.includes(pending.kind)) return null
  const f = pending
  pending = null
  return f
}

/** Nur ansehen, ohne zu verbrauchen. */
export function peekAuthFragment(): AuthFragment | null {
  return pending
}
