import { useEffect, useMemo, useRef, useState } from 'react'
import type { ClipboardEvent, FormEvent } from 'react'
import { ApiError, isDesktopBundle } from '../../lib/api'
import { collectTechInfo, recentErrors } from '../../lib/diagnostics'
import type { TechInfo } from '../../lib/diagnostics'
import { errorText } from '../../lib/errors'
import {
  DESCRIPTION_MAX,
  KIND_LABELS,
  KINDS,
  TITLE_MAX,
  TITLE_MIN,
  feedbackEndpoints,
  prepareScreenshot,
  uploadScreenshot,
} from '../../lib/feedbackApi'
import type { Feedback, FeedbackKind } from '../../lib/feedbackApi'
import { useFeedback } from '../../lib/feedbackStore'
import { useVoice } from '../../lib/voice'
import { useStore } from '../../store'
import { AutoGrowTextarea } from '../AutoGrowTextarea'
import { KindIcon, TechInfoView } from './FeedbackParts'
import { CircleCheck, ImagePlus, X } from 'lucide-react'

const KIND_HINTS: Record<FeedbackKind, string> = {
  bug: 'Etwas funktioniert nicht wie erwartet.',
  suggestion: 'Eine Idee, was Lykos besser machen würde – für alle sichtbar, andere können zustimmen.',
  other: 'Fragen, Lob oder alles andere.',
}

/** Grobe Beschreibung der aktuellen Ansicht – ohne IDs, Namen oder Inhalte. */
function useCurrentArea(): string {
  const dmView = useStore((s) => s.dmView)
  const dm = useStore((s) => s.selectedDmId)
  const community = useStore((s) => s.selectedCommunityId)
  const channel = useStore((s) => s.selectedChannelId)
  const search = useStore((s) => s.search !== null)
  const streams = useVoice((s) => s.watching.length > 0 || s.focus !== null)
  const parts = [
    dmView ? (dm ? 'direktnachricht' : 'direktnachrichten') : channel ? 'community/textkanal' : community ? 'community' : 'start',
  ]
  if (search) parts.push('suche')
  if (streams) parts.push('stream-ansicht')
  return parts.join(' + ')
}

interface ScreenshotDraft {
  blob: Blob
  filename: string
  preview: string
}

export function ReportForm({ onShowMine }: { onShowMine: () => void }) {
  const initialKind = useFeedback((s) => s.initialKind)
  const kindRequest = useFeedback((s) => s.kindRequest)
  const upsertMine = useFeedback((s) => s.upsertMine)
  const area = useCurrentArea()
  const [kind, setKind] = useState<FeedbackKind>(initialKind)
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [attachTech, setAttachTech] = useState(true)
  const [screenshot, setScreenshot] = useState<ScreenshotDraft | null>(null)
  const [screenshotError, setScreenshotError] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<{ feedback: Feedback; screenshotFailed: string | null } | null>(null)
  const fileInput = useRef<HTMLInputElement>(null)

  // Was gesendet würde – zum Zeitpunkt des Öffnens bzw. jeder Änderung neu berechnet.
  const techInfo: TechInfo = useMemo(
    () =>
      collectTechInfo(
        {
          appVersion: __APP_VERSION__,
          desktop: isDesktopBundle,
          userAgent: navigator.userAgent,
          width: window.innerWidth,
          height: window.innerHeight,
          pathname: location.pathname,
          area,
        },
        recentErrors(),
      ),
    [area],
  )

  // „Vorschlag machen“ o. Ä. bei schon offenem Formular: Typ übernehmen (Zustand beim Rendern angleichen).
  const [handledRequest, setHandledRequest] = useState(kindRequest)
  if (handledRequest !== kindRequest) {
    setHandledRequest(kindRequest)
    setKind(initialKind)
  }

  useEffect(() => () => {
    if (screenshot) URL.revokeObjectURL(screenshot.preview)
  }, [screenshot])

  async function takeImage(file: Blob) {
    setScreenshotError(null)
    try {
      const prepared = await prepareScreenshot(file)
      setScreenshot({ ...prepared, preview: URL.createObjectURL(prepared.blob) })
    } catch (e) {
      setScreenshotError(
        e instanceof ApiError
          ? errorText(e)
          : 'Das Bild konnte nicht gelesen werden (PNG, JPEG, WebP oder GIF).',
      )
    }
  }

  function onPaste(e: ClipboardEvent) {
    const image = Array.from(e.clipboardData.files).find((f) => f.type.startsWith('image/'))
    if (!image) return
    e.preventDefault()
    void takeImage(image)
  }

  const titleLen = Array.from(title.trim()).length
  const valid = titleLen >= TITLE_MIN && titleLen <= TITLE_MAX && description.trim().length > 0

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (!valid || busy) return
    setBusy(true)
    setError(null)
    try {
      let feedback = await feedbackEndpoints.create({
        kind,
        title: title.trim(),
        description: description.trim(),
        tech_info: attachTech ? techInfo : null,
      })
      let screenshotFailed: string | null = null
      if (screenshot) {
        try {
          feedback = await uploadScreenshot(feedback.id, screenshot.blob, screenshot.filename)
        } catch (err) {
          screenshotFailed = errorText(err)
        }
      }
      upsertMine(feedback)
      setDone({ feedback, screenshotFailed })
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setDone(null)
    setTitle('')
    setDescription('')
    setScreenshot(null)
    setScreenshotError(null)
  }

  if (done) {
    return (
      <div className="feedback-thanks">
        <CircleCheck size={40} aria-hidden="true" />
        <h3>Danke für deine Meldung!</h3>
        <p>
          Sie ist als <strong>Meldung #{done.feedback.number}</strong> eingegangen. Antworten und Statusänderungen
          findest du unter „Meine Meldungen“.
        </p>
        {done.screenshotFailed && (
          <p className="error">Der Screenshot konnte nicht angehängt werden: {done.screenshotFailed}</p>
        )}
        <div className="form-actions">
          <button type="button" className="button" onClick={reset}>
            Weitere Meldung
          </button>
          <button type="button" className="button primary" onClick={onShowMine}>
            Meine Meldungen
          </button>
        </div>
      </div>
    )
  }

  return (
    <form className="form feedback-form" onSubmit={submit} onPaste={onPaste}>
      <fieldset className="feedback-kinds" aria-label="Art der Meldung">
        {KINDS.map((k) => (
          <label key={k} className={kind === k ? 'active' : ''}>
            <input type="radio" name="kind" value={k} checked={kind === k} onChange={() => setKind(k)} />
            <KindIcon kind={k} size={18} />
            <span>{KIND_LABELS[k]}</span>
          </label>
        ))}
      </fieldset>
      <p className="muted small feedback-kind-hint">{KIND_HINTS[kind]}</p>

      <label className="field">
        <span>Titel</span>
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          maxLength={TITLE_MAX}
          placeholder={kind === 'bug' ? 'z. B. „Nachrichten werden doppelt angezeigt“' : 'Kurz und knapp'}
          autoFocus
          required
        />
      </label>

      <label className="field">
        <span>
          Beschreibung{' '}
          <span className="muted small">(Markdown: **fett**, *kursiv*, `Code`, ```Codeblock```)</span>
        </span>
        <AutoGrowTextarea
          className="feedback-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          maxLength={DESCRIPTION_MAX}
          placeholder={
            kind === 'bug'
              ? 'Was hast du gemacht, was ist passiert, was hättest du erwartet?'
              : 'Beschreibe deine Idee bzw. dein Anliegen.'
          }
          required
        />
      </label>

      <div className="field">
        <span>Screenshot (optional)</span>
        {screenshot ? (
          <div className="feedback-screenshot-draft">
            <img src={screenshot.preview} alt="Vorschau des Screenshots" />
            <button
              type="button"
              className="icon-button small"
              title="Screenshot entfernen"
              onClick={() => setScreenshot(null)}
            >
              <X size={15} />
            </button>
          </div>
        ) : (
          <div className="feedback-screenshot-drop">
            <button type="button" className="button" onClick={() => fileInput.current?.click()}>
              <ImagePlus size={16} /> Bild wählen
            </button>
            <span className="muted small">oder mit Strg+V einfügen. Metadaten (EXIF/GPS) werden entfernt.</span>
          </div>
        )}
        <input
          ref={fileInput}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0]
            e.target.value = ''
            if (file) void takeImage(file)
          }}
        />
        {screenshotError && <p className="error small">{screenshotError}</p>}
        <p className="muted small">Achte darauf, dass auf dem Bild keine privaten Inhalte zu sehen sind.</p>
      </div>

      <div className="feedback-tech-optin">
        <label className="checkbox">
          <input type="checkbox" checked={attachTech} onChange={(e) => setAttachTech(e.target.checked)} />
          Technische Infos anhängen
        </label>
        <details>
          <summary>Was wird gesendet?</summary>
          <p className="muted small">
            Nur wenn der Haken gesetzt ist, und nur an die Admins dieser Instanz: App-Version, Plattform,
            Betriebssystem (grob), Fenstergröße, die aktuelle Ansicht (ohne IDs) und die letzten Fehlermeldungen der
            App. Keine Nachrichteninhalte, keine Namen, keine Zugangsdaten – IDs und Tokens werden geschwärzt.
          </p>
          <TechInfoView info={techInfo} />
        </details>
      </div>

      {error && <p className="error">{error}</p>}
      <div className="form-actions">
        <span className="form-actions-left muted small">Tipp: Strg+Umschalt+F öffnet dieses Fenster.</span>
        <button type="submit" className="button primary" disabled={!valid || busy}>
          {busy ? 'Wird gesendet …' : 'Absenden'}
        </button>
      </div>
    </form>
  )
}
