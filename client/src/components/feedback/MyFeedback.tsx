import { useEffect, useState } from 'react'
import { errorText } from '../../lib/errors'
import { feedbackEndpoints, formatDate } from '../../lib/feedbackApi'
import type { Feedback } from '../../lib/feedbackApi'
import { useFeedback } from '../../lib/feedbackStore'
import { FeedbackText, KindBadge, Replies, Screenshot, StatusBadge, TechInfoView } from './FeedbackParts'
import { ChevronDown, ChevronRight, MessageSquareReply, ThumbsUp, Trash2 } from 'lucide-react'

/** Eigene Meldungen mit Status und Antworten. */
export function MyFeedback({ onNew }: { onNew: () => void }) {
  const mine = useFeedback((s) => s.mine)
  const loadMine = useFeedback((s) => s.loadMine)
  const unseen = useFeedback((s) => s.unseen)
  const markSeen = useFeedback((s) => s.markSeen)
  const [error, setError] = useState<string | null>(null)
  const [open, setOpen] = useState<string | null>(null)
  // Beim Öffnen gemerkte, ungelesene Meldungen bleiben hervorgehoben, bis man das Fenster verlässt.
  const [highlight] = useState(() => new Set(unseen))

  useEffect(() => {
    loadMine().catch((e) => setError(errorText(e)))
    markSeen()
  }, [loadMine, markSeen])

  if (error) return <p className="error">{error}</p>
  if (!mine) return <p className="muted">Wird geladen …</p>
  if (mine.length === 0) {
    return (
      <div className="feedback-empty">
        <p className="muted">Du hast noch nichts gemeldet.</p>
        <button type="button" className="button primary" onClick={onNew}>
          Erste Meldung schreiben
        </button>
      </div>
    )
  }

  return (
    <ul className="feedback-list">
      {mine.map((f) => (
        <MyEntry
          key={f.id}
          feedback={f}
          expanded={open === f.id}
          highlight={highlight.has(f.id)}
          onToggle={() => setOpen(open === f.id ? null : f.id)}
        />
      ))}
    </ul>
  )
}

function MyEntry({
  feedback: f,
  expanded,
  highlight,
  onToggle,
}: {
  feedback: Feedback
  expanded: boolean
  highlight: boolean
  onToggle: () => void
}) {
  const applyDelete = useFeedback((s) => s.applyDelete)
  const [error, setError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState(false)

  async function withdraw() {
    try {
      await feedbackEndpoints.withdraw(f.id)
      applyDelete(f.id)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <li className={`feedback-entry${highlight ? ' highlight' : ''}`}>
      <button type="button" className="feedback-entry-head" aria-expanded={expanded} onClick={onToggle}>
        {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        <span className="feedback-number">#{f.number}</span>
        <span className="feedback-title truncate">{f.title}</span>
        {f.replies.length > 0 && (
          <span className="feedback-meta" title="Antworten">
            <MessageSquareReply size={14} /> {f.replies.length}
          </span>
        )}
        {f.kind === 'suggestion' && (
          <span className="feedback-meta" title="Zustimmungen">
            <ThumbsUp size={14} /> {f.vote_count}
          </span>
        )}
        <StatusBadge status={f.status} />
      </button>
      {expanded && (
        <div className="feedback-entry-body">
          <p className="muted small">
            <KindBadge kind={f.kind} /> gemeldet am {formatDate(f.created_at)}
            {f.updated_at !== f.created_at && <> · zuletzt geändert {formatDate(f.updated_at)}</>}
          </p>
          <FeedbackText text={f.description} />
          {f.screenshot_url && <Screenshot url={f.screenshot_url} />}
          {f.tech_info && (
            <details>
              <summary className="small">Gesendete technische Infos</summary>
              <TechInfoView info={f.tech_info} />
            </details>
          )}
          {f.replies.length > 0 ? (
            <>
              <h4>Antworten</h4>
              <Replies replies={f.replies} />
            </>
          ) : (
            <p className="muted small">Noch keine Antwort.</p>
          )}
          {error && <p className="error">{error}</p>}
          <div className="form-actions">
            {confirm ? (
              <>
                <span className="muted small">Meldung endgültig zurückziehen?</span>
                <button type="button" className="button" onClick={() => setConfirm(false)}>
                  Abbrechen
                </button>
                <button type="button" className="button danger-solid" onClick={() => void withdraw()}>
                  Zurückziehen
                </button>
              </>
            ) : (
              <button type="button" className="button ghost small-button" onClick={() => setConfirm(true)}>
                <Trash2 size={14} /> Zurückziehen
              </button>
            )}
          </div>
        </div>
      )}
    </li>
  )
}
