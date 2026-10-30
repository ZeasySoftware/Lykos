import { useState } from 'react'
import type { FormEvent } from 'react'
import { DEFAULT_SERVER_URL, getServerUrl, isServerChosen, normalizeServerUrl } from '../lib/api'
import type { InstanceInfo } from '../lib/api'
import { useStore } from '../store'
import { Wordmark } from './Brand'
import { PoweredBy } from './PoweredBy'

/** Desktop-Bundle: Instanz wählen (beim ersten Start oder über „Server ändern“ im Anmeldebildschirm). */
export function ServerSetup() {
  const chooseServer = useStore((s) => s.chooseServer)
  const [url, setUrl] = useState(() => (isServerChosen() ? getServerUrl() : DEFAULT_SERVER_URL))
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    const normalized = normalizeServerUrl(url)
    if (!normalized) return setError('Bitte eine gültige Adresse eingeben, z. B. https://lykos.example.org')
    setBusy(true)
    try {
      // Erreichbarkeit prüfen, bevor wir umstellen.
      const res = await fetch(`${normalized}/api/instance`, { credentials: 'omit' })
      if (!res.ok) throw new Error(String(res.status))
      const info = (await res.json()) as InstanceInfo
      if (typeof info.name !== 'string') throw new Error('keine Lykos-Instanz')
      await chooseServer(normalized)
    } catch {
      setError('Unter dieser Adresse antwortet keine Lykos-Instanz.')
      setBusy(false)
    }
  }

  return (
    <div className="auth">
      <form className="auth-card" onSubmit={submit}>
        <div className="auth-brand">
          <Wordmark />
          <h1>Server wählen</h1>
          <p className="muted">Mit welcher Instanz möchtest du dich verbinden?</p>
        </div>
        <label className="field">
          <span>Server-Adresse</span>
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={DEFAULT_SERVER_URL}
            autoFocus
            required
            inputMode="url"
            spellCheck={false}
          />
        </label>
        {error && <p className="error">{error}</p>}
        <button className="button primary" type="submit" disabled={busy}>
          {busy ? 'Verbinde …' : 'Verbinden'}
        </button>
        <p className="muted small">
          Standard ist {DEFAULT_SERVER_URL}. Selbst betriebene Instanzen funktionieren genauso.
        </p>
      </form>
      <PoweredBy />
    </div>
  )
}
