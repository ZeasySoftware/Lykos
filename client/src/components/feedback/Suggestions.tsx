import { useCallback, useEffect, useState } from 'react'
import { ApiError } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { feedbackEndpoints, formatDate } from '../../lib/feedbackApi'
import type { Feedback } from '../../lib/feedbackApi'
import { FeedbackText, Replies, StatusBadge } from './FeedbackParts'
import { ChevronDown, ChevronRight, Lightbulb, ThumbsUp } from 'lucide-react'

const PAGE = 50

/** Vorschläge der Community: alle angemeldeten Nutzer sehen sie und können zustimmen. */
export function Suggestions({ onNew }: { onNew: () => void }) {
  const [sort, setSort] = useState<'votes' | 'new'>('votes')
  const [items, setItems] = useState<Feedback[] | null>(null)
  const [more, setMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [open, setOpen] = useState<string | null>(null)

  const load = useCallback(
    (offset: number, isCancelled: () => boolean = () => false) =>
      feedbackEndpoints
        .suggestions(sort, offset, PAGE)
        .then((page) => {
          if (isCancelled()) return
          setItems((prev) => (offset === 0 ? page : [...(prev ?? []), ...page]))
          setMore(page.length === PAGE)
        })
        .catch((e) => {
          if (!isCancelled()) setError(errorText(e))
        }),
    [sort],
  )

  useEffect(() => {
    let cancelled = false
    void load(0, () => cancelled)
    return () => {
      cancelled = true
    }
  }, [load])

  function changeSort(next: 'votes' | 'new') {
    if (next === sort) return
    setItems(null)
    setSort(next)
  }

  function patch(id: string, change: Partial<Feedback>) {
    setItems((prev) => prev?.map((f) => (f.id === id ? { ...f, ...change } : f)) ?? null)
  }

  async function toggleVote(f: Feedback) {
    try {
      const state = f.voted ? await feedbackEndpoints.unvote(f.id) : await feedbackEndpoints.vote(f.id)
      patch(f.id, state)
    } catch (e) {
      // Bereits abgestimmt (z. B. auf einem anderen Gerät): Stand übernehmen.
      if (e instanceof ApiError && e.code === 'already_voted') patch(f.id, { voted: true })
      else setError(errorText(e))
    }
  }

  return (
    <div className="feedback-suggestions">
      <div className="feedback-toolbar">
        <div className="tabs" role="tablist" aria-label="Sortierung">
          <button type="button" className={sort === 'votes' ? 'active' : ''} onClick={() => changeSort('votes')}>
            Meiste Stimmen
          </button>
          <button type="button" className={sort === 'new' ? 'active' : ''} onClick={() => changeSort('new')}>
            Neueste
          </button>
        </div>
        <button type="button" className="button" onClick={onNew}>
          <Lightbulb size={16} /> Vorschlag machen
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {!items ? (
        <p className="muted">Wird geladen …</p>
      ) : items.length === 0 ? (
        <p className="muted">Noch keine Vorschläge – mach den Anfang!</p>
      ) : (
        <ul className="feedback-list">
          {items.map((f) => {
            const expanded = open === f.id
            return (
              <li key={f.id} className="feedback-entry suggestion">
                <button
                  type="button"
                  className={`feedback-vote${f.voted ? ' voted' : ''}`}
                  aria-pressed={f.voted}
                  title={f.voted ? 'Zustimmung zurücknehmen' : 'Zustimmen (+1)'}
                  onClick={() => void toggleVote(f)}
                >
                  <ThumbsUp size={16} />
                  <span>{f.vote_count}</span>
                </button>
                <div className="feedback-suggestion-main">
                  <button
                    type="button"
                    className="feedback-entry-head"
                    aria-expanded={expanded}
                    onClick={() => setOpen(expanded ? null : f.id)}
                  >
                    {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                    <span className="feedback-title truncate">{f.title}</span>
                    {f.mine && <span className="feedback-mine">von dir</span>}
                    <StatusBadge status={f.status} />
                  </button>
                  {expanded && (
                    <div className="feedback-entry-body">
                      <p className="muted small">
                        #{f.number} · vorgeschlagen am {formatDate(f.created_at)}
                      </p>
                      <FeedbackText text={f.description} />
                      {f.replies.length > 0 && (
                        <>
                          <h4>Antworten</h4>
                          <Replies replies={f.replies} />
                        </>
                      )}
                    </div>
                  )}
                </div>
              </li>
            )
          })}
        </ul>
      )}
      {more && items && (
        <button type="button" className="button ghost" onClick={() => void load(items.length)}>
          Mehr laden
        </button>
      )}
    </div>
  )
}
