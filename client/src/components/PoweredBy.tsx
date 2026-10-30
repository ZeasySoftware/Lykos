import { isDesktopBundle } from '../lib/api'

const ZEASY_URL = 'https://zeasy.software/'

/**
 * „Powered by Zeasy Software“ mit Logo und Link.
 * Im Desktop-Bundle würde ein normaler Link die App-Webview wegnavigieren –
 * dort öffnen wir die Seite im Standardbrowser.
 */
export function PoweredBy({ compact = false }: { compact?: boolean }) {
  async function openExternal(e: React.MouseEvent<HTMLAnchorElement>) {
    if (!isDesktopBundle) return
    e.preventDefault()
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(ZEASY_URL)
  }

  return (
    <a
      className={`powered-by${compact ? ' powered-by-compact' : ''}`}
      href={ZEASY_URL}
      target="_blank"
      rel="noopener noreferrer"
      onClick={openExternal}
      title="Zeasy Software – Dein IT-Dienstleister"
    >
      <span className="powered-by-label">Powered by</span>
      <img className="powered-by-logo" src="/zeasy-logo.svg" alt="Zeasy Software" />
    </a>
  )
}
