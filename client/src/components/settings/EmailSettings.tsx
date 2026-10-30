import { useState } from 'react'
import type { FormEvent } from 'react'
import { Mail } from 'lucide-react'
import { endpoints } from '../../lib/api'
import type { SecurityOverview } from '../../lib/api'
import { confirmAction } from '../../lib/dialogs'
import { errorText } from '../../lib/errors'
import { useStore } from '../../store'
import { emptyReauth, reauthBody } from '../../lib/reauth'
import { ReauthFields } from './ReauthFields'

type Feedback = { ok: boolean; text: string } | null
type Mode = 'idle' | 'set' | 'remove'

/** Optionale E-Mail-Adresse – nur zur Kontowiederherstellung und für Codes per E-Mail. */
export function EmailSettings({ security, onChange }: { security: SecurityOverview; onChange: () => void }) {
  const emailEnabled = useStore((s) => s.instance?.email ?? false)
  const [mode, setMode] = useState<Mode>('idle')
  const [email, setEmail] = useState('')
  const [reauth, setReauth] = useState(emptyReauth)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<Feedback>(null)

  if (!emailEnabled) {
    return (
      <section className="security-section">
        <h3>
          <Mail size={18} aria-hidden="true" /> E-Mail-Adresse
        </h3>
        <p className="muted small">
          Auf dieser Instanz ist kein Mailversand eingerichtet. Ohne E-Mail lässt sich ein vergessenes Passwort nicht
          zurücksetzen – bewahre es gut auf oder richte Wiederherstellungscodes über die Zwei-Faktor-Anmeldung ein.
        </p>
      </section>
    )
  }

  function close() {
    setMode('idle')
    setEmail('')
    setReauth(emptyReauth)
  }

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setMsg(null)
    try {
      if (mode === 'set') {
        const res = await endpoints.setEmail({ email: email.trim(), ...reauthBody(security, reauth) })
        setMsg({
          ok: true,
          text: `Wir haben einen Bestätigungslink an ${res.pending_email} geschickt. Die Adresse gilt erst nach der Bestätigung.`,
        })
      } else {
        await endpoints.deleteEmail(reauthBody(security, reauth))
        setMsg({ ok: true, text: 'E-Mail-Adresse entfernt.' })
      }
      close()
      onChange()
    } catch (err) {
      setMsg({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }

  const hasAny = !!security.email || !!security.pending_email

  return (
    <section className="security-section">
      <h3>
        <Mail size={18} aria-hidden="true" /> E-Mail-Adresse <em className="muted small">(optional)</em>
      </h3>
      <p className="muted small">
        Nur zur Wiederherstellung deines Kontos („Passwort vergessen“) und – wenn du willst – für Bestätigungscodes.
        Andere Nutzer sehen sie nie.
      </p>
      <dl className="security-facts">
        <dt>Bestätigt</dt>
        <dd>{security.email ?? <span className="muted">keine</span>}</dd>
        {security.pending_email && (
          <>
            <dt>Wartet auf Bestätigung</dt>
            <dd>{security.pending_email}</dd>
          </>
        )}
      </dl>
      {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
      {mode === 'idle' ? (
        <div className="form-actions-left">
          <button type="button" className="button" onClick={() => setMode('set')}>
            {security.email ? 'Adresse ändern' : 'Adresse hinterlegen'}
          </button>
          {hasAny && (
            <button
              type="button"
              className="button danger"
              onClick={() =>
                confirmAction({
                  title: 'E-Mail-Adresse entfernen',
                  message: 'Ohne Adresse kannst du dein Passwort nicht mehr per Mail zurücksetzen.',
                  confirmLabel: 'Weiter',
                  action: () => setMode('remove'),
                })
              }
            >
              Entfernen
            </button>
          )}
        </div>
      ) : (
        <form className="form" onSubmit={submit}>
          {mode === 'set' && (
            <label className="field">
              <span>Neue E-Mail-Adresse</span>
              <input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                autoComplete="email"
                maxLength={254}
                required
              />
            </label>
          )}
          {mode === 'remove' && security.mfa.email && (
            <p className="error small">Schalte zuerst die Bestätigung per E-Mail (Zwei-Faktor-Anmeldung) aus.</p>
          )}
          <ReauthFields security={security} value={reauth} onChange={setReauth} />
          <div className="form-actions">
            <button type="button" className="button" onClick={close} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="submit"
              className={mode === 'set' ? 'button primary' : 'button danger-solid'}
              disabled={busy || (mode === 'remove' && security.mfa.email)}
            >
              {mode === 'set' ? 'Bestätigungslink senden' : 'Adresse entfernen'}
            </button>
          </div>
        </form>
      )}
    </section>
  )
}
