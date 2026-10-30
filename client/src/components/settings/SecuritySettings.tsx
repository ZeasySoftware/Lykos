import { useCallback, useEffect, useState } from 'react'
import { endpoints } from '../../lib/api'
import type { SecurityOverview } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { ConnectedAccounts } from './ConnectedAccounts'
import { EmailSettings } from './EmailSettings'
import { TwoFactorSettings } from './TwoFactorSettings'

/** Einstellungsbereich „Sicherheit“: E-Mail, Zwei-Faktor-Anmeldung, verbundene Konten. */
export function SecuritySettings() {
  const [security, setSecurity] = useState<SecurityOverview | null>(null)
  const [error, setError] = useState<string | null>(null)

  const reload = useCallback(() => {
    endpoints
      .security()
      .then((s) => {
        setSecurity(s)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  useEffect(reload, [reload])

  return (
    <div className="form security-settings">
      {error && <p className="error">{error}</p>}
      {!security && !error && <p className="muted">Lade …</p>}
      {security && (
        <>
          <TwoFactorSettings security={security} onChange={reload} />
          <EmailSettings security={security} onChange={reload} />
          <ConnectedAccounts security={security} onChange={reload} />
        </>
      )}
    </div>
  )
}
