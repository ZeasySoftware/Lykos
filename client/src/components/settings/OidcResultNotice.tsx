import { useEffect, useState } from 'react'
import { Link2, X } from 'lucide-react'
import { peekAuthFragment, takeAuthFragment } from '../../lib/authFragment'
import type { AuthFragment } from '../../lib/authFragment'
import { oidcErrorText } from '../../lib/errors'
import { useStore } from '../../store'

function initial(): AuthFragment | null {
  const f = peekAuthFragment()
  return f?.kind === 'linked' || f?.kind === 'error' ? f : null
}

/** Einmaliger Hinweis nach dem Verknüpfen eines Anmeldedienstes (Rückkehr vom Anbieter, angemeldet). */
export function OidcResultNotice() {
  const providers = useStore((s) => s.instance?.oidc_providers)
  const [result, setResult] = useState(initial)

  // Erst nach dem Rendern verbrauchen (StrictMode ruft Initialisierer doppelt auf).
  useEffect(() => {
    takeAuthFragment(['linked', 'error'])
  }, [])

  if (!result) return null
  const ok = result.kind === 'linked'
  const text =
    result.kind === 'linked'
      ? `${providers?.find((p) => p.id === result.provider)?.label ?? result.provider} ist jetzt mit deinem Konto verknüpft.`
      : result.kind === 'error'
        ? oidcErrorText(result.code)
        : ''
  return (
    <div className={`oidc-notice ${ok ? 'success' : 'error'}`} role="status">
      <Link2 size={16} aria-hidden="true" />
      <span>{text}</span>
      <button type="button" className="icon-button small" title="Schließen" onClick={() => setResult(null)}>
        <X size={14} />
      </button>
    </div>
  )
}
