import { useFeedback } from '../../lib/feedbackStore'
import type { FeedbackPanel } from '../../lib/feedbackStore'
import { Modal } from '../Modal'
import { MyFeedback } from './MyFeedback'
import { ReportForm } from './ReportForm'
import { Suggestions } from './Suggestions'

// Feedback-Fenster – eigener Chunk, geladen über `FeedbackHost` (FeedbackHub.tsx).

const TABS: { id: FeedbackPanel; label: string }[] = [
  { id: 'new', label: 'Melden' },
  { id: 'mine', label: 'Meine Meldungen' },
  { id: 'suggestions', label: 'Vorschläge' },
]

/** Feedback-Fenster mit den Reitern „Melden“, „Meine Meldungen“ und „Vorschläge“. */
export function FeedbackHub({ panel }: { panel: FeedbackPanel }) {
  const open = useFeedback((s) => s.open)
  const close = useFeedback((s) => s.close)
  const unseen = useFeedback((s) => s.unseen.length)

  return (
    <Modal title="Feedback" onClose={close} wide>
      <div className="feedback-hub">
        <div className="tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={panel === t.id}
              className={panel === t.id ? 'active' : ''}
              onClick={() => open(t.id)}
            >
              {t.label}
              {t.id === 'mine' && unseen > 0 && panel !== 'mine' && (
                <span className="feedback-dot" aria-label={`${unseen} neue Antworten`} />
              )}
            </button>
          ))}
        </div>
        {/* Formular bleibt beim Reiterwechsel erhalten, damit ein Entwurf nicht verloren geht. */}
        <div hidden={panel !== 'new'}>
          <ReportForm onShowMine={() => open('mine')} />
        </div>
        {panel === 'mine' && <MyFeedback onNew={() => open('new')} />}
        {panel === 'suggestions' && <Suggestions onNew={() => open('new', 'suggestion')} />}
      </div>
    </Modal>
  )
}
