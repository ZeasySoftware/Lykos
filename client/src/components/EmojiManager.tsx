import { useEffect, useRef, useState } from 'react'
import type { DragEvent, KeyboardEvent } from 'react'
import { Check, ImagePlus, Pencil, Trash2, X } from 'lucide-react'
import { endpoints, uploadEmoji } from '../lib/api'
import type { CustomEmoji } from '../lib/api'
import { EMOJI_NAME_RE } from '../lib/customEmoji'
import { useCustomEmojiStore } from '../lib/emojiData'
import { EMOJI_LIMIT, prepareEmoji, suggestEmojiName, uniqueEmojiName } from '../lib/emojiUpload'
import { errorText } from '../lib/errors'
import { displayName } from '../store'
import { CustomEmojiImage } from './CustomEmoji'
import { Avatar } from './Presence'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })
const ACCEPT = 'image/png,image/gif,image/webp,image/jpeg'

/**
 * Verwaltung der Custom-Emotes einer Community: Liste mit Bild, Name (inline umbenennen), Uploader, Datum und
 * Löschen; Upload per Dateiauswahl oder Drag-and-Drop (mehrere Dateien möglich), Anzeige des Limits.
 * `canManage = false` zeigt die Liste nur an.
 */
export function EmojiManager({ communityId, canManage }: { communityId: string; canManage: boolean }) {
  const live = useCustomEmojiStore((s) => s.byCommunity[communityId])
  const [fetched, setFetched] = useState<CustomEmoji[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notes, setNotes] = useState<string[]>([])
  const [busy, setBusy] = useState(0)
  const [dragging, setDragging] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const depth = useRef(0)

  // Eigene Liste laden (z. B. für Instanz-Admins ohne Mitgliedschaft); danach halten Gateway-Events sie aktuell.
  useEffect(() => {
    let cancelled = false
    endpoints.communityEmojis(communityId).then(
      (list) => !cancelled && setFetched(list),
      (e) => !cancelled && setError(errorText(e)),
    )
    return () => {
      cancelled = true
    }
  }, [communityId])

  // Eigene Änderungen sofort zeigen; sobald ein neuerer Gateway-Stand kommt, gilt wieder dieser.
  const [localState, setLocalState] = useState<{ base: CustomEmoji[] | undefined; list: CustomEmoji[] } | null>(null)
  const setLocal = (list: CustomEmoji[]) => setLocalState({ base: live, list })
  const emojis = localState && localState.base === live ? localState.list : (live ?? fetched)
  const full = (emojis?.length ?? 0) >= EMOJI_LIMIT

  async function upload(files: File[]) {
    if (!canManage || files.length === 0) return
    setError(null)
    const problems: string[] = []
    let current = emojis ?? []
    for (const file of files) {
      if (current.length >= EMOJI_LIMIT) {
        problems.push(`${file.name}: Das Limit von ${EMOJI_LIMIT} Emojis ist erreicht.`)
        continue
      }
      setBusy((b) => b + 1)
      try {
        const { blob, filename } = await prepareEmoji(file)
        const name = uniqueEmojiName(
          suggestEmojiName(file.name),
          current.map((e) => e.name),
        )
        const created = await uploadEmoji(communityId, name, blob, filename)
        current = [...current.filter((e) => e.id !== created.id), created].sort((a, b) =>
          a.name.localeCompare(b.name, 'de', { sensitivity: 'base' }),
        )
        setLocal(current)
      } catch (e) {
        problems.push(`${file.name}: ${e instanceof Error && !('status' in e) ? e.message : errorText(e)}`)
      } finally {
        setBusy((b) => b - 1)
      }
    }
    setNotes(problems)
  }

  function replace(updated: CustomEmoji) {
    setLocal((emojis ?? []).map((e) => (e.id === updated.id ? updated : e)))
  }

  function remove(id: string) {
    setLocal((emojis ?? []).filter((e) => e.id !== id))
  }

  const dropProps = canManage
    ? {
        onDragEnter: (e: DragEvent) => {
          if (!Array.from(e.dataTransfer.types).includes('Files')) return
          e.preventDefault()
          depth.current++
          setDragging(true)
        },
        onDragOver: (e: DragEvent) => {
          if (Array.from(e.dataTransfer.types).includes('Files')) e.preventDefault()
        },
        onDragLeave: () => {
          depth.current = Math.max(0, depth.current - 1)
          if (depth.current === 0) setDragging(false)
        },
        onDrop: (e: DragEvent) => {
          e.preventDefault()
          depth.current = 0
          setDragging(false)
          void upload(Array.from(e.dataTransfer.files))
        },
      }
    : {}

  return (
    <div className="form emoji-manager" {...dropProps}>
      <div className="emoji-manager-head">
        <span className={`emoji-manager-count${full ? ' full' : ''}`}>
          {emojis?.length ?? 0} / {EMOJI_LIMIT}
        </span>
      </div>
      <p className="muted small">
        Mitglieder können die Emojis dieser Community überall verwenden – auch in Direktnachrichten und anderen
        Communities. Namen: 2–32 Zeichen aus Buchstaben, Ziffern und _.
      </p>

      {canManage && (
        <div
          className={`emoji-dropzone${dragging ? ' dragging' : ''}${full ? ' disabled' : ''}`}
          role="button"
          tabIndex={full ? -1 : 0}
          aria-disabled={full}
          onClick={() => !full && fileRef.current?.click()}
          onKeyDown={(e) => {
            if (!full && (e.key === 'Enter' || e.key === ' ')) {
              e.preventDefault()
              fileRef.current?.click()
            }
          }}
        >
          <ImagePlus size={22} aria-hidden="true" />
          <span>
            <strong>{busy > 0 ? 'Lade hoch …' : full ? 'Limit erreicht' : 'Emojis hochladen'}</strong>
            <span className="muted small">
              Dateien hierher ziehen oder klicken · PNG, GIF, WebP oder JPEG · Bilder werden auf 128 px verkleinert,
              animierte GIFs müssen schon höchstens 128 px und 256 KB haben.
            </span>
          </span>
          <input
            ref={fileRef}
            type="file"
            accept={ACCEPT}
            multiple
            hidden
            onChange={(e) => {
              const files = Array.from(e.target.files ?? [])
              e.target.value = ''
              void upload(files)
            }}
          />
        </div>
      )}

      {error && <p className="error">{error}</p>}
      {notes.length > 0 && (
        <ul className="error small emoji-manager-notes">
          {notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}

      {emojis === null && !error && <p className="muted small">Lade Emojis …</p>}
      {emojis && emojis.length === 0 && <p className="muted">Noch keine eigenen Emojis.</p>}
      {emojis && emojis.length > 0 && (
        <ul className="emoji-manager-list">
          {emojis.map((e) => (
            <EmojiRow
              key={e.id}
              emoji={e}
              communityId={communityId}
              canManage={canManage}
              onRenamed={replace}
              onDeleted={() => remove(e.id)}
            />
          ))}
        </ul>
      )}
    </div>
  )
}

function EmojiRow({
  emoji,
  communityId,
  canManage,
  onRenamed,
  onDeleted,
}: {
  emoji: CustomEmoji
  communityId: string
  canManage: boolean
  onRenamed: (e: CustomEmoji) => void
  onDeleted: () => void
}) {
  const [editing, setEditing] = useState(false)
  const [name, setName] = useState(emoji.name)
  const [confirm, setConfirm] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const valid = EMOJI_NAME_RE.test(name)

  async function save() {
    if (!valid) return
    if (name === emoji.name) return setEditing(false)
    try {
      onRenamed(await endpoints.renameEmoji(communityId, emoji.id, name))
      setEditing(false)
      setError(null)
    } catch (e) {
      setError(errorText(e))
    }
  }

  async function remove() {
    try {
      await endpoints.deleteEmoji(communityId, emoji.id)
      onDeleted()
    } catch (e) {
      setError(errorText(e))
      setConfirm(false)
    }
  }

  function onKey(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter') {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      e.stopPropagation()
      setName(emoji.name)
      setEditing(false)
    }
  }

  return (
    <li className="emoji-manager-row">
      <span className="emoji-manager-image">
        <CustomEmojiImage id={emoji.id} name={emoji.name} animated={emoji.animated} size={32} tooltip={false} />
      </span>
      <span className="emoji-manager-name">
        {editing ? (
          <input
            value={name}
            onChange={(e) => setName(e.target.value.replace(/\s/g, '_'))}
            onKeyDown={onKey}
            onBlur={() => void save()}
            maxLength={32}
            aria-label="Name des Emojis"
            aria-invalid={!valid}
            autoFocus
          />
        ) : (
          <strong>:{emoji.name}:</strong>
        )}
        {emoji.animated && <span className="muted small">animiert</span>}
        {error && <span className="error small">{error}</span>}
      </span>
      <span className="emoji-manager-uploader">
        {emoji.uploader ? (
          <>
            <Avatar user={emoji.uploader} name={displayName(emoji.uploader)} size={20} />
            <span className="truncate">{displayName(emoji.uploader)}</span>
          </>
        ) : (
          <span className="muted">Gelöschter Nutzer</span>
        )}
        <span className="muted small">{dateFmt.format(new Date(emoji.created_at))}</span>
      </span>
      {canManage && (
        <span className="emoji-manager-actions">
          {confirm ? (
            <>
              <span className="small">Löschen?</span>
              <button type="button" className="icon-button small danger" title="Ja, löschen" onClick={() => void remove()}>
                <Check size={16} />
              </button>
              <button type="button" className="icon-button small" title="Abbrechen" onClick={() => setConfirm(false)}>
                <X size={16} />
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className="icon-button small"
                title="Umbenennen"
                onClick={() => {
                  setName(emoji.name)
                  setEditing(true)
                }}
              >
                <Pencil size={16} />
              </button>
              <button type="button" className="icon-button small danger" title="Löschen" onClick={() => setConfirm(true)}>
                <Trash2 size={16} />
              </button>
            </>
          )}
        </span>
      )}
    </li>
  )
}
