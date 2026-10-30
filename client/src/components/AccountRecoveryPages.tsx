import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { endpoints } from '../lib/api'
import { linkToken } from '../lib/authFragment'
import { errorText } from '../lib/errors'
import { Wordmark } from './Brand'
import { PoweredBy } from './PoweredBy'

/**
 * Token aus `#token=` (bzw. aus älteren Mails `?token=`) lesen und sofort aus der Adresszeile entfernen
 * (landet so nicht im Verlauf oder in Lesezeichen). Einmal pro Seitenaufruf.
 */
let urlToken: string | null | undefined

function useUrlToken(): string | null {
  if (urlToken === undefined) {
    urlToken = linkToken(location.hash, location.search)
    if (urlToken) history.replaceState(null, '', location.pathname)
  }
  return urlToken
}

function Page({ title, children }: { title: string; children: ReactNode }) {
  useEffect(() => {
    document.title = `${title} – Lykos`
  }, [title])
  return (
    <div className="auth">
      <div className="auth-card">
        <div className="auth-brand">
          <Wordmark />
          <h1>{title}</h1>
        </div>
        {children}
      </div>
      <PoweredBy />
    </div>
  )
}

/** Ein Token ist nur einmal einlösbar – doppelte Effekte (StrictMode) teilen sich die Anfrage. */
const verifications = new Map<string, Promise<{ email: string }>>()

function verifyOnce(token: string): Promise<{ email: string }> {
  let p = verifications.get(token)
  if (!p) {
    p = endpoints.verifyEmail(token)
    verifications.set(token, p)
  }
  return p
}

/** `/verify-email#token=…` – E-Mail-Adresse bestätigen (ohne Anmeldung). */
export function VerifyEmailPage() {
  const token = useUrlToken()
  const [state, setState] = useState<{ kind: 'loading' } | { kind: 'ok'; email: string } | { kind: 'error'; text: string }>(
    token ? { kind: 'loading' } : { kind: 'error', text: 'Der Link ist ungültig oder abgelaufen.' },
  )

  useEffect(() => {
    if (!token) return
    let cancelled = false
    verifyOnce(token)
      .then((r) => !cancelled && setState({ kind: 'ok', email: r.email }))
      .catch((e) => !cancelled && setState({ kind: 'error', text: errorText(e) }))
    return () => {
      cancelled = true
    }
  }, [token])

  return (
    <Page title="E-Mail bestätigen">
      {state.kind === 'loading' && <p className="muted">Wird bestätigt …</p>}
      {state.kind === 'ok' && (
        <p className="success">
          Die Adresse <strong>{state.email}</strong> ist jetzt bestätigt. Du kannst darüber dein Passwort
          zurücksetzen.
        </p>
      )}
      {state.kind === 'error' && <p className="error">{state.text}</p>}
      <a className="button" href="/">
        Zu Lykos
      </a>
    </Page>
  )
}
