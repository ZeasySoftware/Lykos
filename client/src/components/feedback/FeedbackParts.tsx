import { useEffect, useState } from 'react'
import { attachmentUrl, attachmentsNeedToken } from '../../lib/api'
import type { TechInfo } from '../../lib/diagnostics'
import { fetchScreenshot, formatDate, KIND_LABELS, STATUS_LABELS } from '../../lib/feedbackApi'
import type { FeedbackKind, FeedbackReply, FeedbackStatus } from '../../lib/feedbackApi'
import { Markdown } from '../../lib/markdown'
import { displayName } from '../../store'
import { Bug, CircleHelp, Lightbulb, Lock } from 'lucide-react'

export function KindIcon({ kind, size = 16 }: { kind: FeedbackKind; size?: number }) {
  if (kind === 'bug') return <Bug size={size} aria-hidden="true" />
  if (kind === 'suggestion') return <Lightbulb size={size} aria-hidden="true" />
  return <CircleHelp size={size} aria-hidden="true" />
}

export function KindBadge({ kind }: { kind: FeedbackKind }) {
  return (
    <span className={`feedback-kind kind-${kind}`}>
      <KindIcon kind={kind} size={13} />
      {KIND_LABELS[kind]}
    </span>
  )
}

export function StatusBadge({ status }: { status: FeedbackStatus }) {
  return <span className={`feedback-status status-${status}`}>{STATUS_LABELS[status]}</span>
}

/** Beschreibung bzw. Antwort als Markdown (gleiche Teilmenge wie Nachrichten). */
export function FeedbackText({ text }: { text: string }) {
  return (
    <div className="feedback-text">
      <Markdown content={text} />
    </div>
  )
}

export function Replies({ replies }: { replies: FeedbackReply[] }) {
  if (replies.length === 0) return null
  return (
    <ul className="feedback-replies">
      {replies.map((r) => (
        <li key={r.id} className={r.internal ? 'internal' : ''}>
          <header>
            <strong>{r.author ? displayName(r.author) : 'Gelöschter Nutzer'}</strong>
            {r.internal && (
              <span className="feedback-internal" title="Nur für Admins sichtbar">
                <Lock size={12} aria-hidden="true" /> Interne Notiz
              </span>
            )}
            <span className="muted small">{formatDate(r.created_at)}</span>
          </header>
          <FeedbackText text={r.body} />
        </li>
      ))}
    </ul>
  )
}

/** Technische Infos als Liste (Melder und Admins). */
export function TechInfoView({ info }: { info: TechInfo }) {
  const rows: [string, string | null][] = [
    ['App-Version', info.app_version],
    ['Plattform', info.platform === 'desktop' ? 'Desktop-App' : info.platform === 'web' ? 'Web' : null],
    ['Betriebssystem', info.os],
    ['Fenstergröße', info.window ? `${info.window.width} × ${info.window.height}` : null],
    ['Ansicht', info.view],
  ]
  return (
    <div className="feedback-tech">
      <dl>
        {rows.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value ?? '–'}</dd>
          </div>
        ))}
      </dl>
      {info.errors.length > 0 ? (
        <details>
          <summary>Letzte Fehlermeldungen ({info.errors.length})</summary>
          <ol className="feedback-errors">
            {info.errors.map((e, i) => (
              <li key={i}>
                {e.at && <span className="muted">{formatDate(e.at)} </span>}
                <code>{e.message}</code>
              </li>
            ))}
          </ol>
        </details>
      ) : (
        <p className="muted small">Keine Fehlermeldungen aufgezeichnet.</p>
      )}
    </div>
  )
}

/** Screenshot einer Meldung (im Desktop-Bundle per fetch mit Token geladen). */
export function Screenshot({ url }: { url: string }) {
  const needsToken = attachmentsNeedToken()
  const [src, setSrc] = useState<string | null>(needsToken ? null : attachmentUrl({ url }))
  const [failed, setFailed] = useState(false)
  const [large, setLarge] = useState(false)

  useEffect(() => {
    if (!needsToken) return
    const ctrl = new AbortController()
    let objectUrl: string | null = null
    fetchScreenshot(url, ctrl.signal)
      .then((blob) => {
        objectUrl = URL.createObjectURL(blob)
        setSrc(objectUrl)
      })
      .catch(() => {
        if (!ctrl.signal.aborted) setFailed(true)
      })
    return () => {
      ctrl.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [url, needsToken])

  if (failed) return <p className="muted small">Screenshot konnte nicht geladen werden.</p>
  if (!src) return <p className="muted small">Screenshot wird geladen …</p>
  return (
    <button
      type="button"
      className={`feedback-screenshot${large ? ' large' : ''}`}
      title={large ? 'Verkleinern' : 'Vergrößern'}
      onClick={() => setLarge(!large)}
    >
      <img src={src} alt="Screenshot zur Meldung" />
    </button>
  )
}
