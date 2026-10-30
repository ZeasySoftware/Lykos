import { useCallback, useEffect, useRef, useState } from 'react'
import { endpoints } from '../lib/api'
import type { Message, MessageChannel } from '../lib/api'
import { errorText } from '../lib/errors'
import { Markdown } from '../lib/markdown'
import type { MarkdownOptions } from '../lib/markdown'
import { useDismiss } from '../lib/useDismiss'
import { displayName, useStore } from '../store'
import { Pin, PinOff, X } from 'lucide-react'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/** Pin-Button im Kanal-Header mit Popover der angepinnten Nachrichten. */
export function PinnedMessages({
  channel,
  canManage,
  markdown,
  onJump,
}: {
  channel: MessageChannel
  canManage: boolean
  markdown: MarkdownOptions
  onJump: (messageId: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [pins, setPins] = useState<Message[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const close = useCallback(() => setOpen(false), [])
  useDismiss(ref, open, close)

  // Beim Öffnen frisch laden; Änderungen per Gateway (MESSAGE_UPDATE mit pinned_at) übernehmen.
  useEffect(() => {
    if (!open) return
    let cancelled = false
    endpoints.pins(channel.id).then(
      (list) => !cancelled && setPins(list),
      (e) => !cancelled && setError(errorText(e)),
    )
    const unsubscribe = useStore.subscribe((s, prev) => {
      const items = s.messages[channel.id]?.items
      if (!items || items === prev.messages[channel.id]?.items) return
      setPins((list) => {
        if (!list) return list
        let next = list
        for (const m of items) {
          const listed = next.some((p) => p.id === m.id)
          if (m.pinned_at && !listed) next = [m, ...next]
          else if (!m.pinned_at && listed) next = next.filter((p) => p.id !== m.id)
        }
        return next === list ? list : next
      })
    })
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [open, channel.id])

  async function unpin(id: string) {
    try {
      await endpoints.unpin(channel.id, id)
      setPins((list) => list?.filter((p) => p.id !== id) ?? null)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <div className="pins" ref={ref}>
      <button
        type="button"
        className="icon-button"
        title="Angepinnte Nachrichten"
        aria-expanded={open}
        onClick={() => {
          if (!open) {
            setPins(null)
            setError(null)
          }
          setOpen(!open)
        }}
      >
        <Pin size={18} />
      </button>
      {open && (
        <div className="pins-popover" role="dialog" aria-label="Angepinnte Nachrichten">
          <header>
            <span className="truncate">Angepinnte Nachrichten</span>
            {/* Mobil ist die Liste eine Vollbild-Ebene – dort braucht sie einen Schließen-Knopf. */}
            <button type="button" className="icon-button mobile-only" onClick={close} aria-label="Schließen">
              <X size={20} />
            </button>
          </header>
          {error && <p className="error small">{error}</p>}
          {!pins && !error && <p className="muted small">Lade …</p>}
          {pins?.length === 0 && <p className="muted small">In diesem Kanal ist nichts angepinnt.</p>}
          <ul>
            {pins?.map((m) => (
              <li key={m.id} className="pin-item">
                <div className="pin-meta">
                  <strong>{displayName(m.author)}</strong>
                  <span className="muted small">{dateFmt.format(new Date(m.created_at))}</span>
                  <span className="pin-actions">
                    <button
                      type="button"
                      className="link-button"
                      onClick={() => {
                        setOpen(false)
                        onJump(m.id)
                      }}
                    >
                      Springen
                    </button>
                    {canManage && (
                      <button type="button" className="icon-button small" title="Lösen" onClick={() => void unpin(m.id)}>
                        <PinOff size={14} />
                      </button>
                    )}
                  </span>
                </div>
                <div className="message-content pin-content">
                  {m.content ? (
                    <Markdown content={m.content} options={{ ...markdown, everyone: m.mention_everyone }} />
                  ) : (
                    <span className="muted">Anhang</span>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
