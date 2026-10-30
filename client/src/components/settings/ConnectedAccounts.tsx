import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link2, Unlink } from 'lucide-react'
import { endpoints, isDesktopBundle } from '../../lib/api'
import type { SecurityOverview } from '../../lib/api'
import { confirmAction } from '../../lib/dialogs'
import { errorText } from '../../lib/errors'
import { useStore } from '../../store'
import { emptyReauth, reauthBody } from '../../lib/reauth'
import { ReauthFields } from './ReauthFields'
import { ProviderIcon } from '../ProviderIcon'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })

/** Verknüpfte Anmeldedienste (OIDC), sofern die Instanz welche anbietet. */
export function ConnectedAccounts({ security, onChange }: { security: SecurityOverview; onChange: () => void }) {
  const providers = useStore((s) => s.instance?.oidc_providers ?? [])
  // Laufende Aktion mit Bestätigungsformular (Passwort/2FA) – Verknüpfen wie Trennen.
  const [pending, setPending] = useState<{ kind: 'link' | 'unlink'; provider: string } | null>(null)
  const [reauth, setReauth] = useState(emptyReauth)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Auch Identitäten von inzwischen entfernten Anbietern anzeigen (zum Trennen).
  const ids = [...new Set([...providers.map((p) => p.id), ...security.identities.map((i) => i.provider)])]
  if (ids.length === 0) return null

  function begin(kind: 'link' | 'unlink', provider: string) {
    setError(null)
    setPending({ kind, provider })
    setReauth(emptyReauth)
  }

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (!pending) return
    setBusy(true)
    setError(null)
    try {
      if (pending.kind === 'link') {
        const { url } = await endpoints.oidcLinkStart(pending.provider, reauthBody(security, reauth))
        window.location.assign(url)
        return
      }
      await endpoints.unlinkIdentity(pending.provider, reauthBody(security, reauth))
      setPending(null)
      setReauth(emptyReauth)
      onChange()
    } catch (err) {
      setError(errorText(err))
    }
    setBusy(false)
  }

  const loginMethods = (security.has_password ? 1 : 0) + security.identities.length

  return (
    <section className="security-section">
      <h3>
        <Link2 size={18} aria-hidden="true" /> Verbundene Konten
      </h3>
      <p className="muted small">
        Melde dich zusätzlich über einen externen Anbieter (z. B. Discord oder GitHub) an. Lykos speichert dabei nur eine
        Kennung des Anbieters – keine Profildaten.
      </p>
      {isDesktopBundle && (
        <p className="notice small">Verknüpfen ist im Browser verfügbar (Web-Version dieser Instanz).</p>
      )}
      {error && <p className="error">{error}</p>}
      <ul className="security-list">
        {ids.map((id) => {
          const identity = security.identities.find((i) => i.provider === id)
          const label = providers.find((p) => p.id === id)?.label ?? identity?.label ?? id
          return (
            <li key={id}>
              <ProviderIcon provider={id} />
              <div className="security-list-text">
                <strong>{label}</strong>
                <span className="muted small">
                  {identity ? `Verknüpft seit ${dateFmt.format(new Date(identity.created_at))}` : 'Nicht verknüpft'}
                </span>
              </div>
              {identity ? (
                <button
                  type="button"
                  className="button danger"
                  disabled={busy || loginMethods <= 1}
                  title={loginMethods <= 1 ? 'Dein einziger Anmeldeweg – lege zuerst ein Passwort fest.' : undefined}
                  onClick={() =>
                    confirmAction({
                      title: 'Verbindung trennen',
                      message: `Die Anmeldung über ${label} funktioniert danach nicht mehr.`,
                      confirmLabel: 'Weiter',
                      action: () => begin('unlink', id),
                    })
                  }
                >
                  <Unlink size={16} aria-hidden="true" /> Trennen
                </button>
              ) : (
                !isDesktopBundle &&
                providers.some((p) => p.id === id) && (
                  <button type="button" className="button" disabled={busy} onClick={() => begin('link', id)}>
                    Verknüpfen
                  </button>
                )
              )}
            </li>
          )
        })}
      </ul>
      {pending && (
        <form className="form security-action" onSubmit={submit}>
          <strong>{pending.kind === 'link' ? 'Konto verknüpfen' : 'Verknüpfung trennen'}</strong>
          <ReauthFields security={security} value={reauth} onChange={setReauth} />
          <div className="form-actions">
            <button type="button" className="button" onClick={() => setPending(null)} disabled={busy}>
              Abbrechen
            </button>
            {pending.kind === 'link' ? (
              <button type="submit" className="button primary" disabled={busy}>
                Weiter zum Anbieter
              </button>
            ) : (
              <button type="submit" className="button danger-solid" disabled={busy}>
                Trennen
              </button>
            )}
          </div>
        </form>
      )}
    </section>
  )
}
