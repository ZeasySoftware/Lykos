import { useCallback, useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { errorText } from '../../lib/errors'
import { openExternalLink } from '../../lib/external'
import {
  KIND_LABELS,
  KINDS,
  REPLY_MAX,
  STATUS_LABELS,
  STATUSES,
  feedbackEndpoints,
  formatDate,
} from '../../lib/feedbackApi'
import type { Feedback, FeedbackKind, FeedbackStatus } from '../../lib/feedbackApi'
import { displayName } from '../../store'
import { AutoGrowTextarea } from '../AutoGrowTextarea'
import {
  FeedbackText,
  KindBadge,
  KindIcon,
  Replies,
  Screenshot,
  StatusBadge,
  TechInfoView,
} from './FeedbackParts'
import { ArrowLeft, ExternalLink, GitBranchPlus, RefreshCw, ThumbsUp, Trash2 } from 'lucide-react'

const PAGE = 50

/**
 * Verwaltung aller Meldungen für Instanz-Admins: Liste mit Filter, Detail mit Screenshot und technischen Infos,
 * Status, Antwort/interne Notiz, Löschen und optional „Als Issue anlegen“ (Gitea).
 */
export function FeedbackAdmin() {
  const [kind, setKind] = useState<FeedbackKind | ''>('')
  const [status, setStatus] = useState<FeedbackStatus | ''>('neu')
  const [items, setItems] = useState<Feedback[] | null>(null)
  const [more, setMore] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const [gitea, setGitea] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(
    (before?: string, isCancelled: () => boolean = () => false) =>
      feedbackEndpoints
        .adminList({ kind: kind || undefined, status: status || undefined, before, limit: PAGE })
        .then((page) => {
          if (isCancelled()) return
          setError(null)
          setItems((prev) => (before ? [...(prev ?? []), ...page] : page))
          setMore(page.length === PAGE)
        })
        .catch((e) => {
          if (!isCancelled()) setError(errorText(e))
        }),
    [kind, status],
  )

  useEffect(() => {
    let cancelled = false
    void load(undefined, () => cancelled)
    return () => {
      cancelled = true
    }
  }, [load])

  useEffect(() => {
    feedbackEndpoints
      .adminSettings()
      .then((s) => setGitea(s.gitea))
      .catch(() => setGitea(false))
  }, [])

  function replace(f: Feedback) {
    setItems((prev) => prev?.map((x) => (x.id === f.id ? f : x)) ?? null)
  }

  function remove(id: string) {
    setItems((prev) => prev?.filter((x) => x.id !== id) ?? null)
    setSelected(null)
  }

  if (selected) {
    return (
      <AdminDetail
        id={selected}
        gitea={gitea}
        onBack={() => setSelected(null)}
        onChanged={replace}
        onDeleted={remove}
      />
    )
  }

  return (
    <div className="feedback-admin">
      <h3>Meldungen</h3>
      <div className="feedback-toolbar">
        <label className="field">
          <span>Typ</span>
          <select value={kind} onChange={(e) => {
              setItems(null)
              setKind(e.target.value as FeedbackKind | '')
            }}>
            <option value="">Alle</option>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {KIND_LABELS[k]}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Status</span>
          <select value={status} onChange={(e) => {
              setItems(null)
              setStatus(e.target.value as FeedbackStatus | '')
            }}>
            <option value="">Alle</option>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_LABELS[s]}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="icon-button" title="Neu laden" onClick={() => void load()}>
          <RefreshCw size={16} />
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {!items ? (
        <p className="muted">Wird geladen …</p>
      ) : items.length === 0 ? (
        <p className="muted">Keine Meldungen für diesen Filter.</p>
      ) : (
        <table className="feedback-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Titel</th>
              <th>Von</th>
              <th>Status</th>
              <th>Eingang</th>
            </tr>
          </thead>
          <tbody>
            {items.map((f) => (
              <tr key={f.id} tabIndex={0} onClick={() => setSelected(f.id)} onKeyDown={(e) => e.key === 'Enter' && setSelected(f.id)}>
                <td className="feedback-number">{f.number}</td>
                <td>
                  <span className="feedback-row-title">
                    <KindIcon kind={f.kind} size={14} />
                    <span className="truncate">{f.title}</span>
                    {f.kind === 'suggestion' && (
                      <span className="feedback-meta" title="Zustimmungen">
                        <ThumbsUp size={12} /> {f.vote_count}
                      </span>
                    )}
                  </span>
                </td>
                <td className="truncate">{f.reporter ? displayName(f.reporter) : <span className="muted">gelöscht</span>}</td>
                <td>
                  <StatusBadge status={f.status} />
                </td>
                <td className="muted small">{formatDate(f.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {more && items && items.length > 0 && (
        <button type="button" className="button ghost" onClick={() => void load(items[items.length - 1].id)}>
          Ältere laden
        </button>
      )}
    </div>
  )
}

function AdminDetail({
  id,
  gitea,
  onBack,
  onChanged,
  onDeleted,
}: {
  id: string
  gitea: boolean
  onBack: () => void
  onChanged: (f: Feedback) => void
  onDeleted: (id: string) => void
}) {
  const [f, setF] = useState<Feedback | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reply, setReply] = useState('')
  const [internal, setInternal] = useState(false)
  const [busy, setBusy] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)

  useEffect(() => {
    feedbackEndpoints
      .adminGet(id)
      .then(setF)
      .catch((e) => setError(errorText(e)))
  }, [id])

  async function run(action: () => Promise<Feedback>) {
    setBusy(true)
    setError(null)
    try {
      const updated = await action()
      setF(updated)
      onChanged(updated)
      return true
    } catch (e) {
      setError(errorText(e))
      return false
    } finally {
      setBusy(false)
    }
  }

  async function sendReply(e: FormEvent) {
    e.preventDefault()
    const text = reply.trim()
    if (!text) return
    if (await run(() => feedbackEndpoints.adminReply(id, text, internal))) setReply('')
  }

  async function remove() {
    setBusy(true)
    try {
      await feedbackEndpoints.adminDelete(id)
      onDeleted(id)
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="feedback-admin-detail">
      <button type="button" className="link-button" onClick={onBack}>
        <ArrowLeft size={14} /> Zurück zur Liste
      </button>
      {error && <p className="error">{error}</p>}
      {!f ? (
        !error && <p className="muted">Wird geladen …</p>
      ) : (
        <>
          <header className="feedback-detail-head">
            <h3>
              <span className="feedback-number">#{f.number}</span> {f.title}
            </h3>
            <p className="muted small">
              <KindBadge kind={f.kind} /> von {f.reporter ? displayName(f.reporter) : 'gelöschtem Nutzer'} ·{' '}
              {formatDate(f.created_at)}
              {f.kind === 'suggestion' && (
                <>
                  {' '}
                  · <ThumbsUp size={12} /> {f.vote_count}
                </>
              )}
            </p>
          </header>

          <div className="feedback-detail-actions">
            <label className="field">
              <span>Status</span>
              <select
                value={f.status}
                disabled={busy}
                onChange={(e) =>
                  void run(() => feedbackEndpoints.adminSetStatus(id, e.target.value as FeedbackStatus))
                }
              >
                {STATUSES.map((s) => (
                  <option key={s} value={s}>
                    {STATUS_LABELS[s]}
                  </option>
                ))}
              </select>
            </label>
            {f.issue_url ? (
              <a
                className="button"
                href={f.issue_url}
                target="_blank"
                rel="noopener noreferrer"
                onClick={(e) => void openExternalLink(e, f.issue_url!)}
              >
                <ExternalLink size={14} /> Issue öffnen
              </a>
            ) : (
              gitea && (
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  title="Titel, Beschreibung und technische Infos an Gitea senden – ohne Nutzernamen und Screenshot"
                  onClick={() => void run(() => feedbackEndpoints.adminCreateIssue(id))}
                >
                  <GitBranchPlus size={14} /> Als Issue anlegen
                </button>
              )
            )}
            {confirmDelete ? (
              <span className="feedback-confirm">
                <button type="button" className="button" onClick={() => setConfirmDelete(false)}>
                  Abbrechen
                </button>
                <button type="button" className="button danger-solid" disabled={busy} onClick={() => void remove()}>
                  Endgültig löschen
                </button>
              </span>
            ) : (
              <button type="button" className="button danger" onClick={() => setConfirmDelete(true)}>
                <Trash2 size={14} /> Löschen
              </button>
            )}
          </div>

          <section>
            <h4>Beschreibung</h4>
            <FeedbackText text={f.description} />
          </section>
          {f.screenshot_url && (
            <section>
              <h4>Screenshot</h4>
              <Screenshot url={f.screenshot_url} />
            </section>
          )}
          <section>
            <h4>Technische Infos</h4>
            {f.tech_info ? (
              <TechInfoView info={f.tech_info} />
            ) : (
              <p className="muted small">Der Melder hat keine technischen Infos angehängt.</p>
            )}
          </section>
          <section>
            <h4>Antworten & Notizen</h4>
            {f.replies.length === 0 && <p className="muted small">Noch keine.</p>}
            <Replies replies={f.replies} />
            <form className="form feedback-reply-form" onSubmit={sendReply}>
              <AutoGrowTextarea
                value={reply}
                onChange={(e) => setReply(e.target.value)}
                maxLength={REPLY_MAX}
                placeholder={internal ? 'Interne Notiz (nur Admins)' : 'Antwort an den Melder (Markdown)'}
              />
              <div className="form-actions">
                <label className="checkbox form-actions-left">
                  <input type="checkbox" checked={internal} onChange={(e) => setInternal(e.target.checked)} />
                  Interne Notiz
                </label>
                <button type="submit" className="button primary" disabled={busy || !reply.trim()}>
                  {internal ? 'Notiz speichern' : 'Antworten'}
                </button>
              </div>
              {!internal && (
                <p className="muted small">
                  {f.reporter
                    ? 'Der Melder sieht die Antwort unter „Meine Meldungen“.'
                    : 'Das Konto des Melders wurde gelöscht.'}
                  {f.kind === 'suggestion' && ' Bei Vorschlägen ist sie für alle sichtbar.'}
                </p>
              )}
            </form>
          </section>
        </>
      )}
    </div>
  )
}
