import { useEffect, useState } from 'react'
import { endpoints } from './api'
import type { MfaMethod, Reauth, SecurityOverview } from './api'

/** Eingaben zur Bestätigung einer sensiblen Aktion (siehe `components/settings/ReauthFields`). */
export interface ReauthValue {
  password: string
  code: string
  method: MfaMethod
}

export const emptyReauth: ReauthValue = { password: '', code: '', method: 'totp' }

/** Verfügbare 2FA-Methoden zur Bestätigung. */
export function reauthMethods(security: SecurityOverview | null): MfaMethod[] {
  if (!security) return []
  const m: MfaMethod[] = []
  if (security.mfa.totp) m.push('totp')
  if (security.mfa.email) m.push('email')
  if (m.length) m.push('recovery')
  return m
}

/** Request-Body aus den Eingaben (nur, was das Konto tatsächlich braucht). */
export function reauthBody(security: SecurityOverview | null, value: ReauthValue): Reauth {
  const body: Reauth = {}
  if (!security || security.has_password) body.password = value.password
  const methods = reauthMethods(security)
  if (methods.length) {
    body.mfa_code = value.code.trim()
    body.mfa_method = methods.includes(value.method) ? value.method : methods[0]
  }
  return body
}

/** Lädt den Sicherheitsstand des Kontos (für Formulare außerhalb der Sicherheits-Einstellungen). */
export function useSecurityOverview(): SecurityOverview | null {
  const [security, setSecurity] = useState<SecurityOverview | null>(null)
  useEffect(() => {
    let cancelled = false
    endpoints
      .security()
      .then((s) => !cancelled && setSecurity(s))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])
  return security
}
