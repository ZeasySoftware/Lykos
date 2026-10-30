import { useEffect, useState } from 'react'
import { Check, Copy, Download, Globe, ShieldAlert } from 'lucide-react'
import { ApiError, endpoints } from '../lib/api'
import type { DesktopRelease } from '../lib/api'
import { Wordmark } from './Brand'
import { ReleaseNotes } from './ReleaseNotes'
import { PoweredBy } from './PoweredBy'

type State = { kind: 'loading' } | { kind: 'ready'; release: DesktopRelease } | { kind: 'error'; text: string }

/** Öffentliche Seite `/download`: Windows-Installer der Desktop-App, ohne Anmeldung erreichbar. */
export function DownloadPage() {
  const [state, setState] = useState<State>({ kind: 'loading' })

  useEffect(() => {
    document.title = 'Lykos herunterladen'
    let cancelled = false
    endpoints
      .desktopLatest()
      .then((release) => !cancelled && setState({ kind: 'ready', release }))
      .catch((e) => {
        if (cancelled) return
        const text =
          e instanceof ApiError && e.status === 404
            ? 'Derzeit ist kein Download verfügbar.'
            : 'Die Download-Informationen konnten nicht geladen werden. Bitte später erneut versuchen.'
        setState({ kind: 'error', text })
      })
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <div className="auth">
      <div className="auth-card download-card">
        <div className="auth-brand">
          <Wordmark />
          <h1>Desktop-App für Windows</h1>
          <p className="muted">Mit globalem Push-to-Talk, Benachrichtigungen und automatischen Updates.</p>
        </div>

        {state.kind === 'loading' && <p className="muted">Lade Download-Informationen …</p>}
        {state.kind === 'error' && <p className="notice">{state.text}</p>}
        {state.kind === 'ready' && <ReleaseInfo release={state.release} />}

        <a className="button ghost download-browser" href="/">
          <Globe size={16} aria-hidden="true" />
          Im Browser nutzen
        </a>
      </div>
      <PoweredBy />
    </div>
  )
}

function ReleaseInfo({ release }: { release: DesktopRelease }) {
  const published = release.published_at ? new Date(release.published_at) : null
  return (
    <>
      <div className="download-meta">
        <span>
          Version <strong>{release.version}</strong>
        </span>
        {published && !Number.isNaN(published.getTime()) && (
          <span className="muted">vom {published.toLocaleDateString('de-DE', { dateStyle: 'long' })}</span>
        )}
        <span className="muted">Windows 10/11 · 64 Bit · {formatSize(release.size)}</span>
      </div>

      <a className="button primary download-button" href={release.download_url} download={release.file_name}>
        <Download size={18} aria-hidden="true" />
        Installer herunterladen
      </a>

      <div className="notice download-smartscreen">
        <ShieldAlert size={18} aria-hidden="true" />
        <div>
          Der Installer ist noch nicht code-signiert. Windows SmartScreen warnt deshalb beim ersten Start: auf{' '}
          <strong>„Weitere Informationen“</strong> und dann <strong>„Trotzdem ausführen“</strong> klicken. Danach
          aktualisiert sich die App selbst (signierte Updates).
        </div>
      </div>

      {release.sha256 && <Checksum fileName={release.file_name} sha256={release.sha256} />}

      {release.notes.trim() && (
        <details className="download-notes">
          <summary>Was ist neu in {release.version}?</summary>
          <div className="download-notes-text">
            <ReleaseNotes text={release.notes} />
          </div>
        </details>
      )}
    </>
  )
}

function Checksum({ fileName, sha256 }: { fileName: string; sha256: string }) {
  const [copied, setCopied] = useState(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(sha256)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      // Zwischenablage nicht verfügbar – der Wert ist ja sichtbar.
    }
  }

  return (
    <div className="field">
      <span>SHA-256</span>
      <div className="copy-row">
        <code className="download-hash">{sha256}</code>
        <button type="button" className="icon-button" onClick={() => void copy()} title="Prüfsumme kopieren">
          {copied ? <Check size={16} aria-label="Kopiert" /> : <Copy size={16} aria-label="Kopieren" />}
        </button>
      </div>
      <p className="muted small">
        Prüfen in PowerShell: <code>Get-FileHash .\{fileName}</code>
      </p>
    </div>
  )
}

function formatSize(bytes: number): string {
  if (!bytes) return 'Größe unbekannt'
  return `${(bytes / (1024 * 1024)).toLocaleString('de-DE', { maximumFractionDigits: 1 })} MB`
}
