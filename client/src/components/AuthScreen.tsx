import { useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { Download, KeyRound, Mail, Smartphone } from 'lucide-react'
import { endpoints, getServerUrl, isDesktopBundle } from '../lib/api'
import type { InvitePreview, MfaMethod, OidcSignupInfo } from '../lib/api'
import { peekAuthFragment, takeAuthFragment } from '../lib/authFragment'
import { errorText, oidcErrorText } from '../lib/errors'
import { extractInviteCode } from '../lib/invite'
import { useStore } from '../store'
import { Wordmark } from './Brand'
import { PoweredBy } from './PoweredBy'
import { LegalLinks } from './LegalLinks'
import { SourceLink } from './SourceLink'
import { ProviderIcon } from './ProviderIcon'
import { InviteCard } from './customize/CommunityView'

type Mode = 'login' | 'register'

/** Was gerade angezeigt wird: Anmelden/Registrieren, „Passwort vergessen“, 2FA-Schritt oder OIDC-Registrierung. */
type View =
  | { kind: 'form' }
  | { kind: 'forgot' }
  | { kind: 'mfa'; token: string; methods: MfaMethod[] }
  | { kind: 'oidc-signup'; token: string }

const FRAGMENT_KINDS = ['signup', 'mfa', 'error'] as const

/** Nur lesen (rein, StrictMode-sicher); verbraucht wird das Fragment im Effekt. */
function initialView(): { view: View; error: string | null } {
  const f = peekAuthFragment()
  if (f?.kind === 'signup') return { view: { kind: 'oidc-signup', token: f.token }, error: null }
  if (f?.kind === 'mfa') return { view: { kind: 'mfa', token: f.token, methods: f.methods }, error: null }
  if (f?.kind === 'error') return { view: { kind: 'form' }, error: oidcErrorText(f.code) }
  return { view: { kind: 'form' }, error: null }
}

export function AuthScreen() {
  const [initial] = useState(initialView)
  const [view, setView] = useState<View>(initial.view)
  useEffect(() => {
    takeAuthFragment([...FRAGMENT_KINDS])
  }, [])

  return (
    <div className="auth">
      {view.kind === 'form' && (
        <AuthForm
          initialError={initial.error}
          onMfa={(token, methods) => setView({ kind: 'mfa', token, methods })}
          onForgot={() => setView({ kind: 'forgot' })}
        />
      )}
      {view.kind === 'forgot' && <ForgotPassword onBack={() => setView({ kind: 'form' })} />}
      {view.kind === 'mfa' && (
        <MfaStep token={view.token} methods={view.methods} onBack={() => setView({ kind: 'form' })} />
      )}
      {view.kind === 'oidc-signup' && <OidcSignup token={view.token} onBack={() => setView({ kind: 'form' })} />}
      <div className="auth-links">
        {!isDesktopBundle && (
          <a className="auth-download" href="/download">
            <Download size={14} aria-hidden="true" />
            Desktop-App herunterladen
          </a>
        )}
        <SourceLink className="auth-download" />
        <LegalLinks className="auth-download" />
      </div>
      <PoweredBy />
    </div>
  )
}

function Brand({ title, subtitle }: { title: string; subtitle?: string }) {
  const instance = useStore((s) => s.instance)
  return (
    <div className="auth-brand">
      <Wordmark />
      <h1>{title}</h1>
      <p className="muted">{subtitle ?? 'VoIP ohne Identitätszwang'}</p>
      {instance && instance.name !== 'Lykos' && (
        <p className="auth-instance">
          Instanz <strong>{instance.name}</strong>
        </p>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Anmelden / Registrieren
// ---------------------------------------------------------------------------

function AuthForm({
  initialError,
  onMfa,
  onForgot,
}: {
  initialError: string | null
  onMfa: (token: string, methods: MfaMethod[]) => void
  onForgot: () => void
}) {
  const instance = useStore((s) => s.instance)
  const instanceError = useStore((s) => s.instanceError)
  const pendingInvite = useStore((s) => s.pendingInvite)
  const login = useStore((s) => s.login)
  const register = useStore((s) => s.register)
  const changeServer = useStore((s) => s.changeServer)

  const [mode, setMode] = useState<Mode>(pendingInvite ? 'register' : 'login')
  const [username, setUsername] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [password2, setPassword2] = useState('')
  const [email, setEmail] = useState('')
  const [invite, setInvite] = useState(pendingInvite ?? '')
  const [error, setError] = useState<string | null>(initialError)
  const [busy, setBusy] = useState(false)
  const preview = useInvitePreview(pendingInvite)

  const bootstrap = instance?.bootstrap ?? false
  const regMode = instance?.registration_mode ?? 'invite_only'
  const registrationClosed = !bootstrap && regMode === 'closed'
  const inviteRequired = !bootstrap && regMode === 'invite_only'
  const emailEnabled = instance?.email ?? false
  // OIDC im Desktop-Client vorerst nicht (bräuchte Systembrowser + Rückkehr in die App).
  const providers = isDesktopBundle ? [] : (instance?.oidc_providers ?? [])

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    if (mode === 'register') {
      if (password !== password2) return setError('Die Passwörter stimmen nicht überein.')
      if (password.length < 10) return setError('Passwort: mindestens 10 Zeichen.')
      if (inviteRequired && !invite.trim()) return setError('Für die Registrierung brauchst du eine Einladung.')
    }
    setBusy(true)
    try {
      if (mode === 'login') {
        const challenge = await login(username.trim(), password)
        if (challenge) onMfa(challenge.mfa_token, challenge.methods)
      } else
        await register({
          username: username.trim(),
          password,
          display_name: displayName.trim() || undefined,
          invite: extractInviteCode(invite) || undefined,
          email: (emailEnabled && email.trim()) || undefined,
        })
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  async function oidcLogin(provider: string) {
    setError(null)
    setBusy(true)
    try {
      const { url } = await endpoints.oidcStart(provider, 'login')
      window.location.assign(url)
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <form className="auth-card" onSubmit={submit}>
      <Brand title={mode === 'login' ? 'Willkommen zurück' : 'Konto erstellen'} />

      {instanceError && <p className="error">Server nicht erreichbar.</p>}

      {isDesktopBundle && (
        <p className="muted small server-line">
          <span className="truncate">Server: {getServerUrl().replace(/^https?:\/\//, '')}</span>
          <button type="button" className="link-button" onClick={changeServer}>
            Ändern
          </button>
        </p>
      )}

      {preview && <InviteCard preview={preview} />}

      <div className="tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={mode === 'login'}
          className={mode === 'login' ? 'active' : ''}
          onClick={() => setMode('login')}
        >
          Anmelden
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode === 'register'}
          className={mode === 'register' ? 'active' : ''}
          onClick={() => setMode('register')}
        >
          Registrieren
        </button>
      </div>

      {mode === 'register' && bootstrap && (
        <p className="notice">Diese Instanz hat noch keine Nutzer. Der erste Nutzer wird Instanz-Admin.</p>
      )}

      {mode === 'register' && registrationClosed ? (
        <p className="notice">Die Registrierung ist auf dieser Instanz geschlossen.</p>
      ) : (
        <>
          <label className="field">
            <span>Benutzername</span>
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              autoFocus
              required
            />
          </label>

          {mode === 'register' && (
            <label className="field">
              <span>
                Anzeigename <em className="muted">(optional)</em>
              </span>
              <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} maxLength={64} />
            </label>
          )}

          {/* Kein <label> um den Block: ein Button darin würde beim Klick auf „Passwort“ ausgelöst. */}
          <div className="field">
            <span className="field-label-row">
              <label htmlFor="auth-password">Passwort</label>
              {mode === 'login' && emailEnabled && (
                <button type="button" className="link-button" onClick={onForgot}>
                  Passwort vergessen?
                </button>
              )}
            </span>
            <input
              id="auth-password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
              required
            />
          </div>

          {mode === 'register' && (
            <>
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
              {emailEnabled && (
                <label className="field">
                  <span>
                    E-Mail <em className="muted">(optional, nur zur Kontowiederherstellung)</em>
                  </span>
                  <input
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    autoComplete="email"
                    maxLength={254}
                  />
                </label>
              )}
              {!bootstrap && (
                <label className="field">
                  <span>
                    Einladung {!inviteRequired && <em className="muted">(optional)</em>}
                  </span>
                  <input
                    value={invite}
                    onChange={(e) => setInvite(e.target.value)}
                    placeholder="Code oder Einladungslink"
                    required={inviteRequired}
                  />
                </label>
              )}
            </>
          )}

          {error && <p className="error">{error}</p>}

          <button className="button primary" type="submit" disabled={busy}>
            {busy ? 'Bitte warten …' : mode === 'login' ? 'Anmelden' : 'Konto erstellen'}
          </button>

          {providers.length > 0 && (
            <div className="auth-oidc">
              <span className="auth-divider muted small">oder</span>
              {providers.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() => void oidcLogin(p.id)}
                >
                  <ProviderIcon provider={p.id} /> Mit {p.label} {mode === 'login' ? 'anmelden' : 'registrieren'}
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </form>
  )
}

// ---------------------------------------------------------------------------
// Passwort vergessen
// ---------------------------------------------------------------------------

function ForgotPassword({ onBack }: { onBack: () => void }) {
  const [login, setLogin] = useState('')
  const [busy, setBusy] = useState(false)
  const [sent, setSent] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await endpoints.forgotPassword(login.trim())
      setSent(true)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="auth-card" onSubmit={submit}>
      <Brand title="Passwort vergessen" subtitle="Wir schicken dir einen Link zum Zurücksetzen." />
      {sent ? (
        <p className="success">
          Wenn ein Konto mit bestätigter E-Mail-Adresse existiert, haben wir dir einen Link geschickt. Er ist 30
          Minuten gültig.
        </p>
      ) : (
        <>
          <label className="field">
            <span>Benutzername oder E-Mail-Adresse</span>
            <input value={login} onChange={(e) => setLogin(e.target.value)} autoComplete="username" autoFocus required />
          </label>
          <p className="muted small">
            Das klappt nur, wenn du in deinem Konto eine E-Mail-Adresse hinterlegt und bestätigt hast.
          </p>
          {error && <p className="error">{error}</p>}
          <button className="button primary" type="submit" disabled={busy || !login.trim()}>
            {busy ? 'Bitte warten …' : 'Link anfordern'}
          </button>
        </>
      )}
      <button type="button" className="link-button" onClick={onBack}>
        Zurück zur Anmeldung
      </button>
    </form>
  )
}

// ---------------------------------------------------------------------------
// Zweiter Anmeldeschritt (2FA)
// ---------------------------------------------------------------------------

const MFA_LABELS: Record<MfaMethod, { label: string; hint: string }> = {
  totp: { label: 'App', hint: 'Gib den 6-stelligen Code aus deiner Authenticator-App ein.' },
  email: { label: 'E-Mail', hint: 'Wir schicken dir einen 6-stelligen Code an deine bestätigte Adresse.' },
  recovery: { label: 'Wiederherstellung', hint: 'Gib einen deiner Wiederherstellungscodes ein (xxxxx-xxxxx).' },
}

const MFA_ICONS: Record<MfaMethod, typeof Smartphone> = { totp: Smartphone, email: Mail, recovery: KeyRound }

function MfaStep({ token, methods, onBack }: { token: string; methods: MfaMethod[]; onBack: () => void }) {
  const completeMfa = useStore((s) => s.completeMfa)
  const [method, setMethod] = useState<MfaMethod>(methods[0] ?? 'recovery')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [mailSent, setMailSent] = useState(false)
  const autoSent = useRef(false)

  async function sendMail() {
    setError(null)
    try {
      await endpoints.loginMfaEmail(token)
      setMailSent(true)
    } catch (err) {
      setError(errorText(err))
    }
  }

  // Ohne Authenticator-App: Code per E-Mail gleich anfordern.
  useEffect(() => {
    if (method === 'email' && !methods.includes('totp') && !autoSent.current) {
      autoSent.current = true
      void sendMail()
    }
  }, [method]) // oxlint-disable-line react-hooks/exhaustive-deps -- nur beim Methodenwechsel

  async function submit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await completeMfa(token, method, code.trim())
    } catch (err) {
      setError(errorText(err))
      setCode('')
      setBusy(false)
    }
  }

  return (
    <form className="auth-card" onSubmit={submit}>
      <Brand title="Bestätigung" subtitle="Dein Konto ist mit einem zweiten Faktor geschützt." />
      {methods.length > 1 && (
        <div className="tabs" role="tablist">
          {methods.map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={method === m}
              className={method === m ? 'active' : ''}
              onClick={() => {
                setMethod(m)
                setCode('')
                setError(null)
              }}
            >
              {MFA_LABELS[m].label}
            </button>
          ))}
        </div>
      )}
      <p className="muted small mfa-hint">
        {(() => {
          const Icon = MFA_ICONS[method]
          return <Icon size={16} aria-hidden="true" />
        })()}
        {MFA_LABELS[method].hint}
      </p>
      {method === 'email' && (
        <div className="form-actions-left">
          <button type="button" className="button small-button" onClick={() => void sendMail()}>
            <Mail size={14} aria-hidden="true" /> {mailSent ? 'Code erneut senden' : 'Code per E-Mail senden'}
          </button>
          {mailSent && <span className="success small">Code verschickt (10 Minuten gültig).</span>}
        </div>
      )}
      <label className="field">
        <span>{method === 'recovery' ? 'Wiederherstellungscode' : 'Code'}</span>
        <input
          key={method}
          className="mfa-code-input"
          value={code}
          onChange={(e) => setCode(e.target.value)}
          inputMode={method === 'recovery' ? 'text' : 'numeric'}
          autoComplete="one-time-code"
          placeholder={method === 'recovery' ? 'xxxxx-xxxxx' : '123456'}
          maxLength={method === 'recovery' ? 16 : 7}
          autoFocus
          required
        />
      </label>
      {error && <p className="error">{error}</p>}
      <button className="button primary" type="submit" disabled={busy || !code.trim()}>
        {busy ? 'Bitte warten …' : 'Bestätigen'}
      </button>
      <button type="button" className="link-button" onClick={onBack}>
        Abbrechen und zurück zur Anmeldung
      </button>
    </form>
  )
}

// ---------------------------------------------------------------------------
// Registrierung über OIDC: Benutzername wählen
// ---------------------------------------------------------------------------

function OidcSignup({ token, onBack }: { token: string; onBack: () => void }) {
  const instance = useStore((s) => s.instance)
  const pendingInvite = useStore((s) => s.pendingInvite)
  const oidcRegister = useStore((s) => s.oidcRegister)
  const [info, setInfo] = useState<OidcSignupInfo | null>(null)
  const [username, setUsername] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [invite, setInvite] = useState(pendingInvite ?? '')
  const [useEmail, setUseEmail] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    endpoints
      .oidcSignupInfo(token)
      .then((i) => !cancelled && setInfo(i))
      .catch((e) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [token])

  const bootstrap = instance?.bootstrap ?? false
  const inviteRequired = !bootstrap && (instance?.registration_mode ?? 'invite_only') === 'invite_only'

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (inviteRequired && !invite.trim()) return setError('Für die Registrierung brauchst du eine Einladung.')
    setBusy(true)
    setError(null)
    try {
      await oidcRegister({
        signup_token: token,
        username: username.trim(),
        display_name: displayName.trim() || undefined,
        invite: extractInviteCode(invite) || undefined,
        use_email: useEmail && !!info?.email,
      })
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <form className="auth-card" onSubmit={submit}>
      <Brand
        title="Benutzername wählen"
        subtitle={
          info
            ? `Du hast dich über ${info.provider_label} angemeldet. Wähle noch einen Namen für Lykos.`
            : 'Fast geschafft.'
        }
      />
      <label className="field">
        <span>Benutzername</span>
        <input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoFocus required />
      </label>
      <label className="field">
        <span>
          Anzeigename <em className="muted">(optional)</em>
        </span>
        <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} maxLength={64} />
      </label>
      {!bootstrap && (
        <label className="field">
          <span>
            Einladung {!inviteRequired && <em className="muted">(optional)</em>}
          </span>
          <input
            value={invite}
            onChange={(e) => setInvite(e.target.value)}
            placeholder="Code oder Einladungslink"
            required={inviteRequired}
          />
        </label>
      )}
      {info?.email && (
        <label className="checkbox">
          <input type="checkbox" checked={useEmail} onChange={(e) => setUseEmail(e.target.checked)} />
          Bestätigte E-Mail {info.email} übernehmen (nur zur Kontowiederherstellung)
        </label>
      )}
      <p className="muted small">Vom Anbieter übernimmt Lykos sonst nichts – kein Name, kein Profilbild.</p>
      {error && <p className="error">{error}</p>}
      <button className="button primary" type="submit" disabled={busy || !info}>
        {busy ? 'Bitte warten …' : 'Konto erstellen'}
      </button>
      <button type="button" className="link-button" onClick={onBack}>
        Abbrechen
      </button>
    </form>
  )
}

function useInvitePreview(code: string | null): InvitePreview | null {
  const [preview, setPreview] = useState<InvitePreview | null>(null)
  useEffect(() => {
    if (!code) return
    let cancelled = false
    endpoints
      .invite(code)
      .then((p) => !cancelled && setPreview(p))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [code])
  return code ? preview : null
}
