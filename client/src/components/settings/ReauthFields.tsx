import { useState } from 'react'
import { Mail } from 'lucide-react'
import { endpoints } from '../../lib/api'
import type { MfaMethod, SecurityOverview } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { reauthMethods } from '../../lib/reauth'
import type { ReauthValue } from '../../lib/reauth'

const METHOD_LABELS: Record<MfaMethod, string> = {
  totp: 'Authenticator-App',
  email: 'Code per E-Mail',
  recovery: 'Wiederherstellungscode',
}

/**
 * Passwort und – bei aktiver 2FA – Bestätigungscode. `mfaOnly`: nur der Code-Teil (wenn das Formular
 * das Passwort selbst abfragt).
 */
export function ReauthFields({
  security,
  value,
  onChange,
  mfaOnly = false,
}: {
  security: SecurityOverview | null
  value: ReauthValue
  onChange: (value: ReauthValue) => void
  mfaOnly?: boolean
}) {
  const methods = reauthMethods(security)
  const method = methods.includes(value.method) ? value.method : methods[0]
  const [mailState, setMailState] = useState<{ ok: boolean; text: string } | null>(null)

  async function sendCode() {
    setMailState(null)
    try {
      await endpoints.sendReauthCode()
      setMailState({ ok: true, text: 'Code verschickt – er ist 10 Minuten gültig.' })
    } catch (e) {
      setMailState({ ok: false, text: errorText(e) })
    }
  }

  return (
    <>
      {!mfaOnly && security?.has_password !== false && (
        <label className="field">
          <span>Aktuelles Passwort</span>
          <input
            type="password"
            value={value.password}
            onChange={(e) => onChange({ ...value, password: e.target.value })}
            autoComplete="current-password"
            required
          />
        </label>
      )}
      {!mfaOnly && security?.has_password === false && (
        <p className="muted small">
          Dein Konto hat kein Passwort. Zur Bestätigung darf deine Anmeldung höchstens 10 Minuten zurückliegen –
          sonst melde dich kurz ab und wieder an.
        </p>
      )}
      {method && (
        <div className="reauth-mfa">
          <div className="field-row">
            <label className="field">
              <span>Bestätigungscode</span>
              <input
                value={value.code}
                onChange={(e) => onChange({ ...value, code: e.target.value, method })}
                inputMode={method === 'recovery' ? 'text' : 'numeric'}
                autoComplete="one-time-code"
                placeholder={method === 'recovery' ? 'xxxxx-xxxxx' : '123456'}
                required
              />
            </label>
            {methods.length > 1 && (
              <label className="field">
                <span>Methode</span>
                <select
                  value={method}
                  onChange={(e) => onChange({ ...value, method: e.target.value as MfaMethod, code: '' })}
                >
                  {methods.map((m) => (
                    <option key={m} value={m}>
                      {METHOD_LABELS[m]}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
          {method === 'email' && (
            <div className="form-actions-left">
              <button type="button" className="button small-button" onClick={() => void sendCode()}>
                <Mail size={14} aria-hidden="true" /> Code per E-Mail senden
              </button>
              {mailState && <span className={`small ${mailState.ok ? 'success' : 'error'}`}>{mailState.text}</span>}
            </div>
          )}
        </div>
      )}
    </>
  )
}
