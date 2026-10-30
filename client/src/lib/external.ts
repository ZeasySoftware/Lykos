import type { MouseEvent } from 'react'
import { isDesktopBundle } from './api'

/**
 * Klick-Handler für externe Links (nur http/https). Im Desktop-Bundle würde ein normaler Link die
 * App-Webview wegnavigieren – dort öffnen wir die Seite über das Opener-Plugin im Standardbrowser.
 * Im Browser übernimmt `target="_blank"` mit `rel="noopener noreferrer"`.
 */
export async function openExternalLink(e: MouseEvent<HTMLAnchorElement>, url: string): Promise<void> {
  if (!isDesktopBundle) return
  e.preventDefault()
  await openExternalUrl(url)
}

/** Externe Adresse (nur http/https) im Standardbrowser bzw. neuen Tab öffnen – z. B. aus einem Kontextmenü. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!/^https?:\/\//i.test(url)) return
  if (!isDesktopBundle) {
    window.open(url, '_blank', 'noopener,noreferrer')
    return
  }
  try {
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(url)
  } catch (err) {
    console.warn('Link konnte nicht geöffnet werden', err)
  }
}
