import { Code } from 'lucide-react'
import { openExternalLink } from '../lib/external'
import { useStore } from '../store'

/** Offizieller Quellcode – falls der Server (älter) keinen nennt. */
const DEFAULT_SOURCE_URL = 'https://github.com/ZeasySoftware/Lykos'

/**
 * Link „Quellcode (AGPL-3.0)“: Die AGPL verlangt, Nutzern eines Netzwerkdienstes den Quellcode der
 * laufenden Fassung anzubieten. Die Adresse kommt von der Instanz (`LYKOS_SOURCE_URL`).
 */
export function SourceLink({ className = 'source-link' }: { className?: string }) {
  const url = useStore((s) => s.instance?.source_url) ?? DEFAULT_SOURCE_URL
  return (
    <a
      className={className}
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(e) => void openExternalLink(e, url)}
      title="Lykos ist freie Software unter der GNU AGPL-3.0"
    >
      <Code size={14} aria-hidden="true" />
      Quellcode (AGPL-3.0)
    </a>
  )
}
