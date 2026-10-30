import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { RotateCcw } from 'lucide-react'
import { adminApi } from '../../lib/adminApi'
import type { AdminSettings, AdminSettingsPatch } from '../../lib/adminApi'
import type { RegistrationMode } from '../../lib/api'
import { errorText } from '../../lib/errors'

const MODES: { value: RegistrationMode; label: string; hint: string }[] = [
  { value: 'open', label: 'Offen', hint: 'Jeder kann sich registrieren.' },
  { value: 'invite_only', label: 'Nur mit Einladung', hint: 'Community- oder Instanz-Einladung nötig.' },
  { value: 'closed', label: 'Geschlossen', hint: 'Keine neuen Konten, auch nicht mit Einladung.' },
]

const ANNOUNCEMENT_MAX = 500

export function SettingsTab() {
  const [settings, setSettings] = useState<AdminSettings | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    adminApi
      .settings()
      .then(setSettings)
      .catch((e) => setError(errorText(e)))
  }, [])

  return (
    <div className="ia-section">
      <div className="ia-section-head">
        <h3>Einstellungen</h3>
      </div>
      <p className="muted small">
        Werte gelten sofort für die ganze Instanz. „Zurücksetzen“ stellt den Standard aus der Server-Konfiguration
        (Umgebungsvariablen) wieder her.
      </p>
      {error && <p className="error">{error}</p>}
      {settings && <SettingsForm key={JSON.stringify(settings)} settings={settings} onSaved={setSettings} />}
    </div>
  )
}

function SettingsForm({ settings, onSaved }: { settings: AdminSettings; onSaved: (s: AdminSettings) => void }) {
  const [name, setName] = useState(settings.instance_name)
  const [mode, setMode] = useState<RegistrationMode>(settings.registration_mode)
  const [maxUpload, setMaxUpload] = useState(String(settings.max_upload_mb))
  const [unlimited, setUnlimited] = useState(settings.user_quota_mb === null)
  const [quota, setQuota] = useState(String(settings.user_quota_mb ?? settings.defaults.user_quota_mb ?? 2048))
  const [allowCreate, setAllowCreate] = useState(settings.allow_community_creation)
  // Ältere Server liefern das Feld nicht – dann gelten Bots als erlaubt (Standard).
  const [allowBots, setAllowBots] = useState(settings.allow_bots ?? true)
  const [announcement, setAnnouncement] = useState(settings.announcement ?? '')
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const [busy, setBusy] = useState(false)

  const isOverridden = (key: string) => settings.overridden.includes(key)

  async function save(patch: AdminSettingsPatch, done = 'Gespeichert.') {
    setBusy(true)
    setMsg(null)
    try {
      onSaved(await adminApi.updateSettings(patch))
      setMsg({ ok: true, text: done })
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) })
      setBusy(false)
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault()
    const upload = Number(maxUpload)
    const quotaMb = Number(quota)
    if (!Number.isInteger(upload) || upload < 1) return setMsg({ ok: false, text: 'Upload-Größe: ganze Zahl ab 1 MB.' })
    if (!unlimited && (!Number.isInteger(quotaMb) || quotaMb < 1))
      return setMsg({ ok: false, text: 'Kontingent: ganze Zahl ab 1 MB oder „unbegrenzt“.' })
    void save({
      instance_name: name.trim(),
      registration_mode: mode,
      max_upload_mb: upload,
      user_quota_mb: unlimited ? 0 : quotaMb,
      allow_community_creation: allowCreate,
      allow_bots: allowBots,
      announcement: announcement.trim() || null,
    })
  }

  function resetButton(field: keyof AdminSettingsPatch, label: string) {
    if (!isOverridden(field)) return null
    return (
      <button
        type="button"
        className="link-button small"
        disabled={busy}
        onClick={() => void save({ [field]: null }, `${label} zurückgesetzt.`)}
      >
        <RotateCcw size={12} aria-hidden="true" /> Zurücksetzen
      </button>
    )
  }

  return (
    <form className="form ia-settings" onSubmit={submit}>
      <label className="field">
        <span className="ia-field-label">
          Name der Instanz {resetButton('instance_name', 'Name')}
        </span>
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={64} required />
        <em className="muted small">Standard: {settings.defaults.instance_name}</em>
      </label>

      <fieldset className="radio-group ia-radio-stack">
        <legend>Registrierung</legend>
        {MODES.map((m) => (
          <label key={m.value}>
            <input type="radio" name="registration_mode" checked={mode === m.value} onChange={() => setMode(m.value)} />
            <span>
              {m.label} <span className="muted small">– {m.hint}</span>
            </span>
          </label>
        ))}
      </fieldset>

      <div className="field-row">
        <label className="field">
          <span className="ia-field-label">
            Max. Upload-Größe (MB) {resetButton('max_upload_mb', 'Upload-Größe')}
          </span>
          <input type="number" min={1} value={maxUpload} onChange={(e) => setMaxUpload(e.target.value)} />
          <em className="muted small">Standard: {settings.defaults.max_upload_mb} MB</em>
        </label>
        <div className="field">
          <span className="ia-field-label">
            Speicher pro Nutzer (MB) {resetButton('user_quota_mb', 'Kontingent')}
          </span>
          <input
            type="number"
            min={1}
            value={quota}
            disabled={unlimited}
            onChange={(e) => setQuota(e.target.value)}
            aria-label="Speicher pro Nutzer in MB"
          />
          <label className="checkbox small">
            <input type="checkbox" checked={unlimited} onChange={(e) => setUnlimited(e.target.checked)} />
            Unbegrenzt
          </label>
          <em className="muted small">
            Standard: {settings.defaults.user_quota_mb === null ? 'unbegrenzt' : `${settings.defaults.user_quota_mb} MB`}
          </em>
        </div>
      </div>

      <label className="checkbox">
        <input type="checkbox" checked={allowCreate} onChange={(e) => setAllowCreate(e.target.checked)} />
        Alle Nutzer dürfen Communities anlegen (sonst nur Instanz-Admins)
      </label>

      <label className="checkbox">
        <input type="checkbox" checked={allowBots} onChange={(e) => setAllowBots(e.target.checked)} />
        Bot-Konten erlauben (aus: niemand kann Bots anlegen oder hinzufügen, alle Bot-Tokens sind gesperrt)
      </label>

      <label className="field">
        <span>
          Ankündigung{' '}
          <span className="muted small">
            ({Array.from(announcement).length}/{ANNOUNCEMENT_MAX}) – erscheint als Banner oben für alle; leer = keine
          </span>
        </span>
        <textarea
          value={announcement}
          onChange={(e) => setAnnouncement(e.target.value)}
          maxLength={ANNOUNCEMENT_MAX}
          rows={3}
          placeholder="z. B. „Heute ab 22 Uhr kurze Wartung.“"
        />
      </label>

      {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
      <div className="form-actions">
        <button type="submit" className="button primary" disabled={busy}>
          Speichern
        </button>
      </div>
    </form>
  )
}
