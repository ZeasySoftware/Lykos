import { useEffect, useState } from 'react'
import { ArrowLeft } from 'lucide-react'
import { ApiError, endpoints } from '../lib/api'
import type { LegalKind } from '../lib/api'
import { Wordmark } from './Brand'
import { MarkdownDocument } from './MarkdownDocument'
import { PoweredBy } from './PoweredBy'

const TITLES: Record<LegalKind, { title: string; missing: string }> = {
  imprint: { title: 'Impressum', missing: 'Diese Instanz hat kein Impressum hinterlegt.' },
  privacy: { title: 'Datenschutzerklärung', missing: 'Diese Instanz hat keine Datenschutzerklärung hinterlegt.' },
}

type State = { kind: 'loading' } | { kind: 'ready'; content: string } | { kind: 'error'; text: string }

/**
 * Öffentliche Seiten `/impressum` und `/datenschutz`: Rechtstexte des Betreibers (Markdown aus
 * `LYKOS_IMPRINT_FILE`/`LYKOS_PRIVACY_FILE`), ohne Anmeldung erreichbar.
 */
export function LegalPage({ kind }: { kind: LegalKind }) {
  const { title, missing } = TITLES[kind]
  const [state, setState] = useState<State>({ kind: 'loading' })

  useEffect(() => {
    document.title = `${title} – Lykos`
    let cancelled = false
    endpoints
      .legal(kind)
      .then(({ content }) => !cancelled && setState({ kind: 'ready', content }))
      .catch((e) => {
        if (cancelled) return
        const text =
          e instanceof ApiError && e.status === 404
            ? missing
            : 'Der Text konnte nicht geladen werden. Bitte später erneut versuchen.'
        setState({ kind: 'error', text })
      })
    return () => {
      cancelled = true
    }
  }, [kind, title, missing])

  return (
    <div className="auth">
      <article className="auth-card legal-card">
        <div className="auth-brand legal-brand">
          <Wordmark />
          <h1>{title}</h1>
        </div>

        {state.kind === 'loading' && <p className="muted">Lade {title} …</p>}
        {state.kind === 'error' && <p className="notice">{state.text}</p>}
        {state.kind === 'ready' && (
          <MarkdownDocument text={state.content} className="legal-text" topHeading={2} keepLineBreaks />
        )}

        <a className="button ghost legal-back" href="/">
          <ArrowLeft size={16} aria-hidden="true" />
          Zurück zu Lykos
        </a>
      </article>
      <PoweredBy />
    </div>
  )
}
