import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { adminApi } from '../../lib/adminApi'
import { errorText } from '../../lib/errors'
import { Wordmark } from '../Brand'
import { PoweredBy } from '../PoweredBy'

/**
 * Öffentliche Seite zum Einlösen eines Passwort-Reset-Links – vom Instanz-Admin erzeugt (24 h) oder per
 * „Passwort vergessen“ als Mail verschickt (30 min).
 * Funktioniert ohne Anmeldung; nach Erfolg sind alle Sitzungen des Kontos beendet.
 */
export function ResetPasswordPage({ token }: { token: string }) {
  const [info, setInfo] = useState<{ username: string; expires_at: string } | null>(null)
  const [invalid, setInvalid] = useState<string | null>(token ? null : 'Der Link ist unvollständig.')
  const [password, setPassword] = useState('')
  const [password2, setPassword2] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState(false)

  useEffect(() => {
    if (!token) return
    let cancelled = false
    adminApi
      .verifyResetToken(token)
      .then((i) => !cancelled && setInfo(i))
      .catch((e) => !cancelled && setInvalid(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [token])

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    if (password !== password2) return setError('Die Passwörter stimmen nicht überein.')
    if (password.length < 10) return setError('Passwort: mindestens 10 Zeichen.')
    setBusy(true)
    try {
      await adminApi.completeReset(token, password)
      setDone(true)
      // Token nicht in der Adresszeile/History stehen lassen.
      history.replaceState(null, '', '/reset-password')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="auth">
      <form className="auth-card" onSubmit={submit}>
        <div className="auth-brand">
          <Wordmark />
          <h1>Passwort zurücksetzen</h1>
        </div>
        {done ? (
          <>
            <p className="notice">Dein Passwort wurde geändert. Alle Geräte wurden abgemeldet.</p>
            <a className="button primary" href="/">
              Zur Anmeldung
            </a>
          </>
        ) : invalid ? (
          <>
            <p className="error">{invalid}</p>
            <p className="muted small">Bitte die Verwaltung der Instanz um einen neuen Link.</p>
            <a className="button" href="/">
              Zur Anmeldung
            </a>
          </>
        ) : !info ? (
          <p className="muted">Link wird geprüft …</p>
        ) : (
          <>
            <p className="muted">
              Neues Passwort für <strong>@{info.username}</strong>
            </p>
            {/* Für Passwort-Manager: Benutzername mitschicken. */}
            <input type="text" autoComplete="username" value={info.username} readOnly hidden />
            <label className="field">
              <span>Neues Passwort</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="new-password"
                autoFocus
                required
              />
            </label>
            <label className="field">
              <span>Passwort wiederholen</span>
              <input
                type="password"
                value={password2}
                onChange={(e) => setPassword2(e.target.value)}
                autoComplete="new-password"
                required
              />
            </label>
            {error && <p className="error">{error}</p>}
            <button className="button primary" type="submit" disabled={busy}>
              {busy ? 'Bitte warten …' : 'Passwort speichern'}
            </button>
          </>
        )}
      </form>
      <PoweredBy />
    </div>
  )
}
