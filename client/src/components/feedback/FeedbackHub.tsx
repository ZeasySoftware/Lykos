import { Suspense, useEffect } from 'react'
import { useFeedback } from '../../lib/feedbackStore'
import { lazyComponent } from '../../lib/lazy'
import { useStore } from '../../store'
import { MessageSquareWarning } from 'lucide-react'

/** Das Fenster (Formular, eigene Meldungen, Vorschläge) wird erst beim Öffnen geladen. */
const FeedbackHub = lazyComponent(() => import('./FeedbackWindow').then((m) => m.FeedbackHub))

/** Strg+Umschalt+F (macOS: ⌘+Umschalt+F) – kollidiert mit keinem Browser-Standardkürzel. */
function isFeedbackShortcut(e: KeyboardEvent): boolean {
  return (e.ctrlKey || e.metaKey) && e.shiftKey && !e.altKey && e.code === 'KeyF'
}

/**
 * Einmal in der App eingehängt: Tastenkürzel, Fenster und Zurücksetzen beim Kontowechsel.
 */
export function FeedbackHost() {
  const panel = useFeedback((s) => s.panel)
  const userId = useStore((s) => s.user?.id)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isFeedbackShortcut(e)) return
      e.preventDefault()
      const { panel, open, close } = useFeedback.getState()
      if (panel) close()
      else open('new')
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Anderes Konto (oder abgemeldet): eigene Meldungen nicht weiter anzeigen.
  useEffect(() => {
    useFeedback.getState().reset()
  }, [userId])

  // Ohne Platzhalter: Das Fenster ist ein Modal und erscheint, sobald der (meist vorgeladene) Chunk da ist.
  return panel ? (
    <Suspense fallback={null}>
      <FeedbackHub panel={panel} />
    </Suspense>
  ) : null
}

/** Knopf in der Nutzerleiste; Punkt bei neuen Antworten auf eigene Meldungen. */
export function FeedbackButton() {
  const open = useFeedback((s) => s.open)
  const unseen = useFeedback((s) => s.unseen.length)
  return (
    <button
      type="button"
      className="icon-button feedback-button"
      title={unseen > 0 ? 'Feedback – neue Antworten (Strg+Umschalt+F)' : 'Feedback geben (Strg+Umschalt+F)'}
      aria-label="Feedback"
      onClick={() => open(unseen > 0 ? 'mine' : 'new')}
    >
      <MessageSquareWarning size={18} />
      {unseen > 0 && <span className="feedback-dot" aria-hidden="true" />}
    </button>
  )
}
