import { useState } from 'react'
import type { FormEvent } from 'react'
import { Check, Copy, Download, KeyRound, Mail, ShieldCheck, Smartphone } from 'lucide-react'
import { endpoints } from '../../lib/api'
import type { SecurityOverview, TotpSetup } from '../../lib/api'
import { confirmAction } from '../../lib/dialogs'
import { errorText } from '../../lib/errors'
import { useStore } from '../../store'
import { emptyReauth, reauthBody } from '../../lib/reauth'
import { ReauthFields } from './ReauthFields'

type Action = 'totp-setup' | 'totp-disable' | 'email-enable' | 'email-disable' | 'regen'
type Feedback = { ok: boolean; text: string } | null

const ACTION_TITLES: Record<Action, string> = {
  'totp-setup': 'Authenticator-App einrichten',
  'totp-disable': 'Authenticator-App entfernen',
  'email-enable': 'Codes per E-Mail einschalten',
  'email-disable': 'Codes per E-Mail ausschalten',
  regen: 'Neue Wiederherstellungscodes erzeugen',
}

/** Zwei-Faktor-Anmeldung: Authenticator-App (TOTP), Codes per E-Mail, Wiederherstellungscodes. */
export function TwoFactorSettings({ security, onChange }: { security: SecurityOverview; onChange: () => void }) {
  const emailEnabled = useStore((s) => s.instance?.email ?? false)
  const [action, setAction] = useState<Action | null>(null)
  const [reauth, setReauth] = useState(emptyReauth)
  const [setup, setSetup] = useState<TotpSetup | null>(null)
  const [code, setCode] = useState('')
  const [codes, setCodes] = useState<string[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<Feedback>(null)

  const { mfa } = security
  const active = mfa.totp || mfa.email

  function start(a: Action) {
    setAction(a)
    setReauth(emptyReauth)
    setSetup(null)
    setCode('')
    setMsg(null)
  }

  function cancel() {
    setAction(null)
    setSetup(null)
    setCode('')
  }

  async function run<T>(fn: () => Promise<T>, done: (result: T) => void) {
    setBusy(true)
    setMsg(null)
    try {
      done(await fn())
    } catch (err) {
      setMsg({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }

  function submitReauth(e: FormEvent) {
    e.preventDefault()
    const body = reauthBody(security, reauth)
    switch (action) {
      case 'totp-setup':
        return run(() => endpoints.totpSetup(body), setSetup)
      case 'totp-disable':
        return run(
          () => endpoints.totpDisable(body),
          () => finish('Authenticator-App entfernt.'),
        )
      case 'email-enable':
        return run(
          () => endpoints.emailMfaEnable(body),
          (r) => finish('Bestätigung per E-Mail eingeschaltet.', r.recovery_codes),
        )
      case 'email-disable':
        return run(
          () => endpoints.emailMfaDisable(body),
          () => finish('Bestätigung per E-Mail ausgeschaltet.'),
        )
      case 'regen':
        return run(
          () => endpoints.regenerateRecoveryCodes(body),
          (r) => finish('Neue Wiederherstellungscodes erzeugt – die alten gelten nicht mehr.', r.recovery_codes),
        )
    }
  }

  function submitTotpCode(e: FormEvent) {
    e.preventDefault()
    void run(
      () => endpoints.totpEnable(code.trim()),
      (r) => finish('Authenticator-App eingerichtet.', r.recovery_codes),
    )
  }

  function finish(text: string, recoveryCodes?: string[] | null) {
    setAction(null)
    setSetup(null)
    setCode('')
    setMsg({ ok: true, text })
    if (recoveryCodes) setCodes(recoveryCodes)
    onChange()
  }

  if (codes) return <RecoveryCodesPanel codes={codes} onDone={() => setCodes(null)} />

  return (
    <section className="security-section">
      <h3>
        <ShieldCheck size={18} aria-hidden="true" /> Zwei-Faktor-Anmeldung{' '}
        {active ? <span className="badge">Aktiv</span> : <em className="muted small">(optional)</em>}
      </h3>
      <p className="muted small">
        Zusätzlich zum Passwort fragt Lykos bei der Anmeldung nach einem Code. Damit nützt ein erratenes oder
        geleaktes Passwort allein nichts mehr.
      </p>
      {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}

      {action === null && (
        <ul className="security-list">
          <li>
            <Smartphone size={18} aria-hidden="true" />
            <div className="security-list-text">
              <strong>Authenticator-App</strong>
              <span className="muted small">
                {mfa.totp ? 'Eingerichtet' : 'Codes aus einer App wie Aegis, 2FAS oder einem Passwortmanager'}
              </span>
            </div>
            {mfa.totp ? (
              <button
                type="button"
                className="button danger"
                onClick={() =>
                  confirmAction({
                    title: 'Authenticator-App entfernen',
                    message: 'Die Anmeldung fragt dann keinen Code aus der App mehr ab. Im nächsten Schritt bestätigst du mit Passwort und Code.',
                    confirmLabel: 'Weiter',
                    action: () => start('totp-disable'),
                  })
                }
              >
                Entfernen
              </button>
            ) : (
              <button type="button" className="button" onClick={() => start('totp-setup')}>
                Einrichten
              </button>
            )}
          </li>
          {emailEnabled && (
            <li>
              <Mail size={18} aria-hidden="true" />
              <div className="security-list-text">
                <strong>Code per E-Mail</strong>
                <span className="muted small">
                  {mfa.email
                    ? 'Eingeschaltet'
                    : security.email
                      ? `Codes an ${security.email}`
                      : 'Braucht eine bestätigte E-Mail-Adresse'}
                </span>
              </div>
              {mfa.email ? (
                <button
                  type="button"
                  className="button danger"
                  onClick={() =>
                    confirmAction({
                      title: 'Bestätigung per E-Mail ausschalten',
                      message: 'Codes per E-Mail werden dann nicht mehr als zweiter Faktor akzeptiert.',
                      confirmLabel: 'Weiter',
                      action: () => start('email-disable'),
                    })
                  }
                >
                  Ausschalten
                </button>
              ) : (
                <button
                  type="button"
                  className="button"
                  onClick={() => start('email-enable')}
                  disabled={!security.email}
                >
                  Einschalten
                </button>
              )}
            </li>
          )}
          {active && (
            <li>
              <KeyRound size={18} aria-hidden="true" />
              <div className="security-list-text">
                <strong>Wiederherstellungscodes</strong>
                <span className="muted small">
                  {mfa.recovery_codes_left === 1
                    ? 'Noch 1 unbenutzter Code'
                    : `Noch ${mfa.recovery_codes_left} unbenutzte Codes`}{' '}
                  – für den Fall, dass du keinen Zugriff auf App oder E-Mail hast.
                </span>
              </div>
              <button
                type="button"
                className="button"
                onClick={() =>
                  confirmAction({
                    title: 'Wiederherstellungscodes neu erzeugen',
                    message: 'Alle bisherigen Wiederherstellungscodes werden ungültig.',
                    confirmLabel: 'Weiter',
                    action: () => start('regen'),
                  })
                }
              >
                Neu erzeugen
              </button>
            </li>
          )}
        </ul>
      )}

      {action !== null && !setup && (
        <form className="form security-action" onSubmit={submitReauth}>
          <strong>{ACTION_TITLES[action]}</strong>
          <p className="muted small">Bitte bestätige, dass du es bist.</p>
          <ReauthFields security={security} value={reauth} onChange={setReauth} />
          <div className="form-actions">
            <button type="button" className="button" onClick={cancel} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="submit"
              className={action.endsWith('disable') ? 'button danger-solid' : 'button primary'}
              disabled={busy}
            >
              Weiter
            </button>
          </div>
        </form>
      )}

      {setup && (
        <form className="form security-action" onSubmit={submitTotpCode}>
          <strong>Authenticator-App einrichten</strong>
          <ol className="security-steps small">
            <li>Scanne den QR-Code mit deiner Authenticator-App – oder gib den Schlüssel von Hand ein.</li>
            <li>Gib zur Bestätigung den 6-stelligen Code ein, den die App anzeigt.</li>
          </ol>
          <div className="totp-setup">
            <img
              className="totp-qr"
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(setup.qr_svg)}`}
              alt="QR-Code für die Authenticator-App"
              width={200}
              height={200}
            />
            <div className="totp-secret">
              <span className="muted small">Schlüssel</span>
              <code>{setup.secret.replace(/(.{4})/g, '$1 ').trim()}</code>
              <CopyButton text={setup.secret} label="Schlüssel kopieren" />
            </div>
          </div>
          <label className="field">
            <span>Code aus der App</span>
            <input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              inputMode="numeric"
              autoComplete="one-time-code"
              placeholder="123456"
              maxLength={7}
              autoFocus
              required
            />
          </label>
          <div className="form-actions">
            <button type="button" className="button" onClick={cancel} disabled={busy}>
              Abbrechen
            </button>
            <button type="submit" className="button primary" disabled={busy || code.trim().length < 6}>
              Aktivieren
            </button>
          </div>
        </form>
      )}
    </section>
  )
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Zwischenablage nicht verfügbar – der Text ist ja sichtbar.
    }
  }
  return (
    <button type="button" className="button small-button" onClick={() => void copy()}>
      {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}{' '}
      {copied ? 'Kopiert' : label}
    </button>
  )
}

/** Einmalige Anzeige neuer Wiederherstellungscodes. */
function RecoveryCodesPanel({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const instance = useStore((s) => s.instance)
  const username = useStore((s) => s.user?.username)
  const [saved, setSaved] = useState(false)
  const text = [
    `Lykos – Wiederherstellungscodes${username ? ` für ${username}` : ''}${instance ? ` (${instance.name})` : ''}`,
    'Jeder Code funktioniert genau einmal.',
    '',
    ...codes,
    '',
  ].join('\n')

  function download() {
    const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }))
    const link = document.createElement('a')
    link.href = url
    link.download = `lykos-wiederherstellungscodes${username ? `-${username}` : ''}.txt`
    link.click()
    setTimeout(() => URL.revokeObjectURL(url), 60_000)
  }

  return (
    <section className="security-section">
      <h3>
        <KeyRound size={18} aria-hidden="true" /> Deine Wiederherstellungscodes
      </h3>
      <p className="notice small">
        Bewahre diese Codes sicher auf, z. B. im Passwortmanager. Mit jedem Code kannst du dich einmal anmelden, wenn
        du keinen Zugriff auf deine App oder E-Mail hast. Sie werden <strong>nur jetzt</strong> angezeigt.
      </p>
      <ul className="recovery-codes">
        {codes.map((c) => (
          <li key={c}>
            <code>{c}</code>
          </li>
        ))}
      </ul>
      <div className="form-actions-left">
        <CopyButton text={codes.join('\n')} label="Kopieren" />
        <button type="button" className="button small-button" onClick={download}>
          <Download size={14} aria-hidden="true" /> Als Textdatei herunterladen
        </button>
      </div>
      <label className="checkbox">
        <input type="checkbox" checked={saved} onChange={(e) => setSaved(e.target.checked)} />
        Ich habe die Codes gesichert
      </label>
      <div className="form-actions">
        <button type="button" className="button primary" disabled={!saved} onClick={onDone}>
          Fertig
        </button>
      </div>
    </section>
  )
}
