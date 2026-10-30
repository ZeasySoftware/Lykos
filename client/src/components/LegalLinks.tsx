import { Scale, ShieldCheck } from 'lucide-react'
import { getServerUrl, isDesktopBundle } from '../lib/api'
import type { LegalAvailability } from '../lib/api'
import { openExternalLink } from '../lib/external'
import { useStore } from '../store'

/** Seiten der Rechtstexte (siehe `LegalPage.tsx`); im Desktop-Bundle auf dem Server der Instanz. */
const LEGAL_PATHS = { imprint: '/impressum', privacy: '/datenschutz' } as const

/**
 * Links „Impressum“ und „Datenschutz“ – nur die, die die Instanz hinterlegt hat (`instance.legal`).
 * Im Desktop-Bundle öffnen sie die Seite der Instanz im Standardbrowser, damit die App-Webview nicht
 * wegnavigiert; `newTab` öffnet sie im Web in einem neuen Tab (z. B. aus den Einstellungen heraus).
 */
export function LegalLinks({
  className,
  legal,
  newTab = false,
}: {
  className: string
  /** Ohne Angabe aus dem Store (Download-Seite lädt die Instanzdaten selbst). */
  legal?: LegalAvailability
  newTab?: boolean
}) {
  const fromStore = useStore((s) => s.instance?.legal)
  const available = legal ?? fromStore
  if (!available) return null
  const links = [
    available.imprint && { path: LEGAL_PATHS.imprint, label: 'Impressum', Icon: Scale },
    available.privacy && { path: LEGAL_PATHS.privacy, label: 'Datenschutz', Icon: ShieldCheck },
  ].filter((link) => !!link)

  return links.map(({ path, label, Icon }) => {
    const url = isDesktopBundle ? `${getServerUrl()}${path}` : path
    return (
      <a
        key={path}
        className={className}
        href={url}
        {...(newTab || isDesktopBundle ? { target: '_blank', rel: 'noopener noreferrer' } : {})}
        onClick={(e) => void openExternalLink(e, url)}
      >
        <Icon size={14} aria-hidden="true" />
        {label}
      </a>
    )
  })
}
