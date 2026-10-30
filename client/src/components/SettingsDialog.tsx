import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints, fetchExport, isDesktopBundle } from '../lib/api'
import type { FriendRequestPolicy, Session } from '../lib/api'
import { useAppSettings } from '../lib/appSettings'
import { getAutostart, hasDesktopShell, setAutostart } from '../lib/desktop'
import type { SettingsTab } from '../lib/dialogs'
import { confirmAction } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { listMutes, setChannelMuted, setCommunityMuted, useMutesVersion } from '../lib/notify'
import { toastError } from '../lib/toast'
import { displayName, useStore } from '../store'
import { PoweredBy } from './PoweredBy'
import { LegalLinks } from './LegalLinks'
import { SourceLink } from './SourceLink'
import { Avatar } from './Presence'
import { SettingsLayer, SettingsPage, SettingsSection } from './SettingsLayer'
import type { SettingsGroup } from './SettingsLayer'
import { ProfileAppearance } from './customize/ProfileAppearance'
import { useAdminPanel } from './admin/adminPanel'
import { SoundSettings } from './settings/SoundSettings'
import { SecuritySettings } from './settings/SecuritySettings'
import { ReauthFields } from './settings/ReauthFields'
import { emptyReauth, reauthBody, useSecurityOverview } from '../lib/reauth'
import { StorageTab } from './StorageUsage'
import { SwitchRow } from './Switch'
import { VoiceSettingsTab } from './VoiceSettingsTab'
import { BellOff, Download, LogOut, Monitor, Shield } from 'lucide-react'

type Section = SettingsTab | 'logout' | 'instance-admin'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

type Feedback = { ok: boolean; text: string } | null

const GROUPS: SettingsGroup<Section>[] = [
  {
    title: 'Nutzereinstellungen',
    items: [
      { id: 'account', label: 'Mein Konto' },
      { id: 'security', label: 'Sicherheit' },
      { id: 'profile', label: 'Profil' },
      { id: 'sessions', label: 'Sitzungen & Geräte' },
      { id: 'privacy', label: 'Datenschutz & Daten' },
      { id: 'storage', label: 'Speicher' },
    ],
  },
  {
    title: 'App-Einstellungen',
    items: [
      { id: 'voice', label: 'Sprache & Video' },
      { id: 'sounds', label: 'Sounds' },
      { id: 'notifications', label: 'Benachrichtigungen' },
      { id: 'desktop', label: 'Desktop-App', hidden: !hasDesktopShell },
      { id: 'advanced', label: 'Erweitert' },
    ],
  },
  // Vorläufig: Instanz-Verwaltung nur für Instanz-Admins (später eigener Admin-Bereich).
  {
    title: 'Instanz',
    items: [{ id: 'instance-admin', label: 'Instanz-Verwaltung', icon: <Shield size={16} /> }],
  },
  { items: [{ id: 'logout', label: 'Abmelden', danger: true, icon: <LogOut size={16} /> }] },
]

/** Nutzer-Einstellungen als Vollbild-Ebene. */
export function SettingsDialog({ onClose, initialTab = 'account' }: { onClose: () => void; initialTab?: SettingsTab }) {
  const logout = useStore((s) => s.logout)
  const isAdmin = useStore((s) => s.user?.is_instance_admin ?? false)
  const groups = isAdmin ? GROUPS : GROUPS.filter((g) => g.title !== 'Instanz')
  // „Darstellung“ gibt es nicht als eigenen Bereich – ältere Verweise landen bei „Erweitert“.
  const [tab, setTab] = useState<SettingsTab>(initialTab === 'appearance' ? 'advanced' : initialTab)

  function onAction(id: Section): boolean {
    if (id === 'instance-admin') {
      // Eigene Vollbild-Ebene (Paket H) – Einstellungen dafür schließen.
      onClose()
      useAdminPanel.getState().show()
      return true
    }
    if (id !== 'logout') return false
    confirmAction({
      title: 'Abmelden',
      message: 'Willst du dich auf diesem Gerät wirklich abmelden?',
      confirmLabel: 'Abmelden',
      action: () => logout(),
    })
    return true
  }

  return (
    <SettingsLayer<Section>
      label="Einstellungen"
      groups={groups}
      active={tab}
      onSelect={(id) => id !== 'logout' && id !== 'instance-admin' && setTab(id)}
      onAction={onAction}
      onClose={onClose}
      sidebarFooter={
        <div className="settings-about">
          <span className="muted small">Lykos {__APP_VERSION__}</span>
          <span className="muted small">{isDesktopBundle ? 'Desktop-App' : 'Web'}</span>
          <SourceLink />
          <LegalLinks className="source-link" newTab />
          <PoweredBy compact />
        </div>
      }
    >
      {tab === 'account' && <AccountPage onEditProfile={() => setTab('profile')} />}
      {tab === 'security' && (
        <SettingsPage
          title="Sicherheit"
          description="Alles freiwillig: E-Mail zur Kontowiederherstellung, Zwei-Faktor-Anmeldung und verbundene Konten."
        >
          <SecuritySettings />
        </SettingsPage>
      )}
      {tab === 'profile' && (
        <SettingsPage title="Profil" description="So sehen dich andere – rechts die Vorschau mit deinen noch nicht gespeicherten Angaben.">
          <ProfileAppearance />
        </SettingsPage>
      )}
      {tab === 'sessions' && <SessionsPage />}
      {tab === 'privacy' && <PrivacyPage />}
      {tab === 'storage' && (
        <SettingsPage title="Speicher">
          <StorageTab />
        </SettingsPage>
      )}
      {tab === 'voice' && (
        <SettingsPage title="Sprache & Video">
          <VoiceSettingsTab />
        </SettingsPage>
      )}
      {tab === 'sounds' && (
        <SettingsPage title="Sounds" description="Soundeffekte für Nachrichten, Erwähnungen und Sprachkanäle – nur auf diesem Gerät.">
          <SoundSettings />
        </SettingsPage>
      )}
      {tab === 'notifications' && <NotificationsPage />}
      {tab === 'desktop' && hasDesktopShell && <DesktopPage />}
      {tab === 'advanced' && <AdvancedPage />}
    </SettingsLayer>
  )
}

// ---------------------------------------------------------------------------
// Mein Konto
// ---------------------------------------------------------------------------

function AccountPage({ onEditProfile }: { onEditProfile: () => void }) {
  const user = useStore((s) => s.user)
  return (
    <SettingsPage title="Mein Konto">
      <div className="account-card">
        <div className="account-card-banner" />
        <div className="account-card-head">
          <Avatar user={user} name={displayName(user)} size={80} className="account-card-avatar" />
          <div className="account-card-names">
            <strong>{displayName(user)}</strong>
            <span className="muted">@{user?.username}</span>
          </div>
          <button type="button" className="button primary" onClick={onEditProfile}>
            Profil bearbeiten
          </button>
        </div>
        <dl className="account-card-fields">
          <div>
            <dt>Anzeigename</dt>
            <dd>{user?.display_name || <span className="muted">Nicht gesetzt</span>}</dd>
          </div>
          <div>
            <dt>Benutzername</dt>
            <dd>{user?.username}</dd>
          </div>
          <div>
            <dt>Dabei seit</dt>
            <dd>{user ? dateFmt.format(new Date(user.created_at)) : ''}</dd>
          </div>
        </dl>
      </div>
      <PasswordForm />
    </SettingsPage>
  )
}

function PasswordForm() {
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [repeat, setRepeat] = useState('')
  const [logoutOthers, setLogoutOthers] = useState(true)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<Feedback>(null)
  // 2FA-Code bzw. Konten ohne Passwort (nur OIDC): siehe settings/ReauthFields.
  const security = useSecurityOverview()
  const [reauth, setReauth] = useState(emptyReauth)
  const hasPassword = security?.has_password !== false

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (next !== repeat) {
      setMsg({ ok: false, text: 'Die neuen Passwörter stimmen nicht überein.' })
      return
    }
    setBusy(true)
    setMsg(null)
    try {
      const { mfa_code, mfa_method } = reauthBody(security, reauth)
      await endpoints.changePassword({
        current_password: hasPassword ? current : undefined,
        new_password: next,
        logout_other_sessions: logoutOthers,
        mfa_code,
        mfa_method,
      })
      setReauth(emptyReauth)
      setCurrent('')
      setNext('')
      setRepeat('')
      setMsg({
        ok: true,
        text: logoutOthers ? 'Passwort geändert. Alle anderen Sitzungen wurden abgemeldet.' : 'Passwort geändert.',
      })
    } catch (err) {
      setMsg({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <SettingsSection
      title={hasPassword ? 'Passwort ändern' : 'Passwort festlegen'}
      description={
        hasPassword
          ? 'Mindestens 10 Zeichen. Das aktuelle Passwort (und ggf. dein 2FA-Code) bestätigt die Änderung.'
          : 'Dein Konto nutzt bisher nur die Anmeldung über einen Anbieter. Mit einem Passwort hast du einen zweiten Anmeldeweg.'
      }
    >
      <form className="form" onSubmit={submit}>
        {hasPassword ? (
          <label className="field">
            <span>Aktuelles Passwort</span>
            <input
              type="password"
              value={current}
              onChange={(e) => setCurrent(e.target.value)}
              autoComplete="current-password"
              required
            />
          </label>
        ) : (
          <ReauthFields security={security} value={reauth} onChange={setReauth} />
        )}
        <div className="field-row">
          <label className="field">
            <span>Neues Passwort</span>
            <input
              type="password"
              value={next}
              onChange={(e) => setNext(e.target.value)}
              autoComplete="new-password"
              minLength={10}
              required
            />
          </label>
          <label className="field">
            <span>Neues Passwort wiederholen</span>
            <input
              type="password"
              value={repeat}
              onChange={(e) => setRepeat(e.target.value)}
              autoComplete="new-password"
              minLength={10}
              required
            />
          </label>
        </div>
        <label className="checkbox">
          <input type="checkbox" checked={logoutOthers} onChange={(e) => setLogoutOthers(e.target.checked)} />
          Alle anderen Sitzungen abmelden
        </label>
        {hasPassword && <ReauthFields security={security} value={reauth} onChange={setReauth} mfaOnly />}
        {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
        <div className="form-actions form-actions-start">
          <button type="submit" className="button primary" disabled={busy}>
            {hasPassword ? 'Passwort ändern' : 'Passwort festlegen'}
          </button>
        </div>
      </form>
    </SettingsSection>
  )
}

// ---------------------------------------------------------------------------
// Sitzungen & Geräte
// ---------------------------------------------------------------------------

function SessionsPage() {
  const [sessions, setSessions] = useState<Session[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    endpoints
      .sessions()
      .then((s) => !cancelled && setSessions(s))
      .catch((e) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [])

  function revoke(s: Session) {
    confirmAction({
      title: 'Sitzung widerrufen',
      message: `„${s.label ?? 'Unbekanntes Gerät'}“ wird abgemeldet und muss sich neu anmelden.`,
      confirmLabel: 'Widerrufen',
      action: async () => {
        await endpoints.revokeSession(s.id)
        setSessions((list) => list?.filter((x) => x.id !== s.id) ?? null)
      },
    })
  }

  function revokeOthers() {
    confirmAction({
      title: 'Alle anderen Sitzungen abmelden',
      message: 'Alle Geräte außer diesem werden abgemeldet.',
      confirmLabel: 'Alle abmelden',
      action: async () => {
        await endpoints.revokeOtherSessions()
        setSessions((list) => list?.filter((s) => s.current) ?? null)
      },
    })
  }

  const current = sessions?.find((s) => s.current)
  const others = sessions?.filter((s) => !s.current) ?? []

  return (
    <SettingsPage
      title="Sitzungen & Geräte"
      description="Hier bist du angemeldet. Lykos speichert keine IP-Adressen; Sitzungen laufen 30 Tage nach der letzten Nutzung ab."
    >
      {error && <p className="error">{error}</p>}
      {!sessions && !error && <p className="muted">Lade …</p>}
      {current && (
        <SettingsSection title="Aktuelles Gerät">
          <SessionRow session={current} />
        </SettingsSection>
      )}
      {sessions && (
        <SettingsSection title="Andere Geräte">
          {others.length === 0 ? (
            <p className="muted small">Du bist auf keinem anderen Gerät angemeldet.</p>
          ) : (
            <ul className="session-list">
              {others.map((s) => (
                <SessionRow key={s.id} session={s} onRevoke={() => revoke(s)} />
              ))}
            </ul>
          )}
        </SettingsSection>
      )}
      {others.length > 0 && (
        <SettingsSection title="Alle anderen abmelden" description="Hilfreich, wenn du ein Gerät verloren hast.">
          <div className="form-actions form-actions-start">
            <button type="button" className="button danger" onClick={revokeOthers}>
              Alle anderen Sitzungen abmelden ({others.length})
            </button>
          </div>
        </SettingsSection>
      )}
    </SettingsPage>
  )
}

function SessionRow({ session: s, onRevoke }: { session: Session; onRevoke?: () => void }) {
  return (
    <li className="session">
      <span className="session-icon" aria-hidden="true">
        <Monitor size={20} />
      </span>
      <div className="session-text">
        <strong>{s.label ?? 'Unbekanntes Gerät'}</strong>
        {s.current && <span className="badge">Diese Sitzung</span>}
        <div className="muted small">
          Angemeldet {dateFmt.format(new Date(s.created_at))} · zuletzt aktiv {dateFmt.format(new Date(s.last_used_at))}
        </div>
      </div>
      {onRevoke && (
        <button type="button" className="button danger small-button" onClick={onRevoke}>
          Widerrufen
        </button>
      )}
    </li>
  )
}

// ---------------------------------------------------------------------------
// Datenschutz & Daten
// ---------------------------------------------------------------------------

function PrivacyPage() {
  return (
    <SettingsPage title="Datenschutz & Daten">
      <DmPrivacy />
      <FriendRequestPrivacy />
      <ExportBox />
      <DeleteAccountBox />
    </SettingsPage>
  )
}

function DmPrivacy() {
  const [allow, setAllow] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    endpoints
      .dmSettings()
      .then((s) => setAllow(s.allow_dms))
      .catch((e) => setError(errorText(e)))
  }, [])

  async function change(value: boolean) {
    setError(null)
    setAllow(value)
    try {
      setAllow((await endpoints.updateDmSettings(value)).allow_dms)
    } catch (e) {
      setAllow(!value)
      setError(errorText(e))
    }
  }

  return (
    <SettingsSection title="Direktnachrichten">
      <SwitchRow
        title="Direktnachrichten von Community-Mitgliedern erlauben"
        description="Ausgeschaltet kann dir niemand eine neue Unterhaltung beginnen. Bestehende Unterhaltungen bleiben erhalten, und du kannst selbst weiterhin anderen schreiben."
        checked={allow ?? false}
        disabled={allow === null}
        onChange={(v) => void change(v)}
      />
      {error && <p className="error">{error}</p>}
    </SettingsSection>
  )
}

const FRIEND_REQUEST_OPTIONS: { value: FriendRequestPolicy; label: string; description: string }[] = [
  { value: 'everyone', label: 'Alle', description: 'Jeder auf dieser Instanz, der deinen Benutzernamen kennt.' },
  {
    value: 'mutual_communities',
    label: 'Mitglieder gemeinsamer Communities',
    description: 'Nur Personen, mit denen du mindestens eine Community teilst.',
  },
  { value: 'nobody', label: 'Niemand', description: 'Du kannst weiterhin selbst Anfragen schicken.' },
]

function FriendRequestPrivacy() {
  const [policy, setPolicy] = useState<FriendRequestPolicy | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    endpoints
      .relationshipSettings()
      .then((s) => setPolicy(s.friend_requests))
      .catch((e) => setError(errorText(e)))
  }, [])

  async function change(value: FriendRequestPolicy) {
    const prev = policy
    setError(null)
    setPolicy(value)
    try {
      setPolicy((await endpoints.updateRelationshipSettings(value)).friend_requests)
    } catch (e) {
      setPolicy(prev)
      setError(errorText(e))
    }
  }

  return (
    <SettingsSection
      title="Freundschaftsanfragen"
      description="Wer darf dir Freundschaftsanfragen schicken? Offene Anfragen bleiben bestehen. Freunde können dir auch ohne gemeinsame Community schreiben; blockierte Personen nie."
    >
      <div className="radio-group friends-radio-stack" role="radiogroup" aria-label="Wer darf dir Freundschaftsanfragen schicken?">
        {FRIEND_REQUEST_OPTIONS.map((o) => (
          <label key={o.value}>
            <input
              type="radio"
              name="friend-requests"
              checked={policy === o.value}
              disabled={policy === null}
              onChange={() => void change(o.value)}
            />
            <span>
              <strong>{o.label}</strong>
              <span className="muted small"> – {o.description}</span>
            </span>
          </label>
        ))}
      </div>
      {error && <p className="error">{error}</p>}
    </SettingsSection>
  )
}

function ExportBox() {
  const [busy, setBusy] = useState(false)
  const username = useStore((s) => s.user?.username)

  async function download() {
    setBusy(true)
    try {
      const blob = await fetchExport()
      const url = URL.createObjectURL(blob)
      const link = document.createElement('a')
      link.href = url
      link.download = `lykos-export-${username ?? 'konto'}-${new Date().toISOString().slice(0, 10)}.json`
      link.click()
      setTimeout(() => URL.revokeObjectURL(url), 60_000)
    } catch (e) {
      toastError(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <SettingsSection
      title="Deine Daten"
      description="Lade alles herunter, was Lykos über dich speichert: Profil, Sitzungen, Mitgliedschaften, deine Nachrichten (mit Kanal und Community), Angaben zu deinen Dateianhängen, Freunde, Anfragen und Blockierungen sowie E-Mail-Adresse, 2FA-Status und verbundene Konten (ohne Geheimnisse) – als JSON-Datei."
    >
      <div className="form-actions form-actions-start">
        <button type="button" className="button" onClick={() => void download()} disabled={busy}>
          <Download size={16} aria-hidden="true" /> {busy ? 'Wird erstellt …' : 'Datenexport herunterladen'}
        </button>
      </div>
    </SettingsSection>
  )
}

function DeleteAccountBox() {
  const user = useStore((s) => s.user)
  const communities = useStore((s) => s.communities)
  const logout = useStore((s) => s.logout)
  const [open, setOpen] = useState(false)
  const [password, setPassword] = useState('')
  const [deleteMessages, setDeleteMessages] = useState(false)
  const [confirmName, setConfirmName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const security = useSecurityOverview()
  const [reauth, setReauth] = useState(emptyReauth)
  const hasPassword = security?.has_password !== false

  const owned = communities.filter((c) => c.owner_id === user?.id)

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (!user || confirmName !== user.username) return
    setBusy(true)
    setError(null)
    try {
      await endpoints.deleteAccount({
        ...reauthBody(security, { ...reauth, password }),
        delete_messages: deleteMessages,
      })
      // Session ist serverseitig schon weg – lokal aufräumen.
      await logout()
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <SettingsSection
      title="Konto löschen"
      danger
      description="Löscht dein Konto mit Profil, Profilbild, Sitzungen und Mitgliedschaften endgültig. Deine Nachrichten bleiben als „Gelöschter Nutzer“ stehen – oder werden auf Wunsch mitgelöscht. Das lässt sich nicht rückgängig machen."
    >
      {owned.length > 0 && (
        <p className="notice small">
          Du besitzt noch {owned.length === 1 ? 'die Community' : 'die Communities'}{' '}
          {owned.map((c) => `„${c.name}“`).join(', ')}. Übertrage den Besitz oder lösche sie vorher
          (Community-Einstellungen → Gefahrenzone).
        </p>
      )}
      {!open ? (
        <div className="form-actions form-actions-start">
          <button type="button" className="button danger" onClick={() => setOpen(true)} disabled={owned.length > 0}>
            Konto löschen …
          </button>
        </div>
      ) : (
        <form className="form danger-box" onSubmit={submit}>
          <label className="checkbox">
            <input type="checkbox" checked={deleteMessages} onChange={(e) => setDeleteMessages(e.target.checked)} />
            Auch alle meine Nachrichten und Anhänge löschen
          </label>
          {hasPassword && (
            <label className="field">
              <span>Passwort</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                required
              />
            </label>
          )}
          <ReauthFields security={security} value={reauth} onChange={setReauth} mfaOnly={hasPassword} />
          <label className="field">
            <span>
              Zur Bestätigung deinen Benutzernamen eingeben: <em>{user?.username}</em>
            </span>
            <input value={confirmName} onChange={(e) => setConfirmName(e.target.value)} autoComplete="off" />
          </label>
          {error && <p className="error">{error}</p>}
          <div className="form-actions">
            <button type="button" className="button ghost" onClick={() => setOpen(false)} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="submit"
              className="button danger-solid"
              disabled={busy || (hasPassword && !password) || confirmName !== user?.username}
            >
              Konto endgültig löschen
            </button>
          </div>
        </form>
      )}
    </SettingsSection>
  )
}

// ---------------------------------------------------------------------------
// Benachrichtigungen
// ---------------------------------------------------------------------------

function NotificationsPage() {
  useMutesVersion()
  const details = useStore((s) => s.details)
  const dms = useStore((s) => s.dms)
  const me = useStore((s) => s.user)
  const mutes = listMutes()

  function nameOf(kind: 'channel' | 'community', id: string): string {
    if (kind === 'community') return details[id]?.community.name ?? 'Unbekannte Community'
    for (const d of Object.values(details)) {
      const c = d.channels.find((x) => x.id === id)
      if (c) return `${c.kind === 'text' ? '#' : ''}${c.name} · ${d.community.name}`
    }
    const dm = dms[id]
    if (dm) {
      const others = dm.recipients.filter((u) => u.id !== me?.id).map((u) => displayName(u))
      return dm.name ?? (others.join(', ') || 'Direktnachricht')
    }
    return 'Unbekannter Kanal'
  }

  return (
    <SettingsPage
      title="Benachrichtigungen"
      description={
        isDesktopBundle
          ? 'Die Desktop-App meldet neue Nachrichten, solange ihr Fenster nicht im Vordergrund ist. Erwähnungen kommen auch aus stummgeschalteten Kanälen – außer bei „Bitte nicht stören“.'
          : 'Desktop-Benachrichtigungen gibt es in der Desktop-App. Im Browser wirkt Stummschalten trotzdem: stummgeschaltete Kanäle und Communities treten in den Listen zurück.'
      }
    >
      <SettingsSection
        title="Stummgeschaltet"
        description="Stummschalten geht per Rechtsklick auf einen Kanal, eine Community oder eine Direktnachricht."
      >
        {mutes.length === 0 ? (
          <p className="muted small">Nichts stummgeschaltet.</p>
        ) : (
          <ul className="session-list">
            {mutes.map((m) => (
              <li key={`${m.kind}:${m.id}`} className="session">
                <span className="session-icon" aria-hidden="true">
                  <BellOff size={18} />
                </span>
                <div className="session-text">
                  <strong className="truncate">{nameOf(m.kind, m.id)}</strong>
                  <div className="muted small">
                    {m.kind === 'community' ? 'Community' : 'Kanal'} · {m.label ?? 'bis du es änderst'}
                  </div>
                </div>
                <button
                  type="button"
                  className="button small-button"
                  onClick={() => (m.kind === 'community' ? setCommunityMuted(m.id, false) : setChannelMuted(m.id, false))}
                >
                  Aufheben
                </button>
              </li>
            ))}
          </ul>
        )}
      </SettingsSection>
    </SettingsPage>
  )
}

// ---------------------------------------------------------------------------
// Desktop-App (nur im Tauri-Fenster)
// ---------------------------------------------------------------------------

const isWindows = /Windows/.test(navigator.userAgent)

function DesktopPage() {
  const closeToTray = useAppSettings((s) => s.closeToTray)
  const update = useAppSettings((s) => s.update)
  const [autostart, setAutostartState] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let alive = true
    getAutostart()
      .then((on) => alive && setAutostartState(on))
      .catch(() => alive && setAutostartState(false))
    return () => {
      alive = false
    }
  }, [])

  async function toggleAutostart(on: boolean) {
    setBusy(true)
    try {
      await setAutostart(on)
      setAutostartState(on)
    } catch (e) {
      toastError(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <SettingsPage title="Desktop-App" description="Verhalten des Fensters – nur auf diesem Gerät.">
      <SettingsSection title="Fenster">
        <SwitchRow
          title="Beim Schließen in den Infobereich minimieren"
          description="Das X versteckt Lykos nur: Die App bleibt im Infobereich der Taskleiste erreichbar, Benachrichtigungen kommen weiter und ein laufender Sprachanruf bleibt verbunden. Ganz beenden über „Beenden“ im Menü des Tray-Symbols."
          checked={closeToTray}
          onChange={(v) => update({ closeToTray: v })}
        />
      </SettingsSection>
      <SettingsSection title="Systemstart">
        <SwitchRow
          title={isWindows ? 'Mit Windows starten' : 'Beim Anmelden starten'}
          description="Lykos startet beim Anmelden minimiert im Infobereich."
          checked={autostart ?? false}
          disabled={autostart === null || busy}
          onChange={(v) => void toggleAutostart(v)}
        />
      </SettingsSection>
    </SettingsPage>
  )
}

// ---------------------------------------------------------------------------
// Erweitert
// ---------------------------------------------------------------------------

function AdvancedPage() {
  const developerMode = useAppSettings((s) => s.developerMode)
  const reduceMotion = useAppSettings((s) => s.reduceMotion)
  const update = useAppSettings((s) => s.update)
  return (
    <SettingsPage title="Erweitert">
      <SettingsSection title="Entwicklung">
        <SwitchRow
          title="Entwicklermodus"
          description="Zeigt in den Kontextmenüs „ID kopieren“ für Nachrichten, Nutzer, Kanäle und Communities – praktisch für Fehlerberichte und die API."
          checked={developerMode}
          onChange={(v) => update({ developerMode: v })}
        />
      </SettingsSection>
      <SettingsSection title="Barrierefreiheit">
        <SwitchRow
          title="Animationen reduzieren"
          description="Schaltet Ein- und Ausblendungen weitgehend ab. Die Systemeinstellung „Bewegung reduzieren“ gilt unabhängig davon immer."
          checked={reduceMotion}
          onChange={(v) => update({ reduceMotion: v })}
        />
      </SettingsSection>
    </SettingsPage>
  )
}
