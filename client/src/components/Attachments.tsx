import { useEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'
import { INLINE_IMAGE_TYPES, attachmentUrl, attachmentsNeedToken, fetchAttachment } from '../lib/api'
import type { Attachment } from '../lib/api'
import { errorText } from '../lib/errors'
import { downloadAttachment } from '../lib/download'
import { useAddFiles } from '../lib/fileInput'
import { useEscapeLayer } from '../lib/layers'
import { formatBytes, useUploads } from '../lib/uploads'
import { useStore } from '../store'
import { Download, FileText, Paperclip, TriangleAlert, X } from 'lucide-react'

/** Maximale Vorschaugröße von Bildern im Verlauf (px). */
const PREVIEW_MAX_W = 400
const PREVIEW_MAX_H = 300

const isImage = (a: Attachment) => INLINE_IMAGE_TYPES.includes(a.content_type)

/**
 * URL, unter der ein Anhang im `<img>` bzw. Download-Link nutzbar ist.
 * Web: direkt (Cookie). Desktop-Bundle: per fetch mit Token laden und als Object-URL bereitstellen.
 */
function useAttachmentSrc(a: Attachment, enabled = true): { src: string | null; failed: boolean } {
  const direct = attachmentsNeedToken() ? null : attachmentUrl(a)
  const [loaded, setLoaded] = useState<{ url: string; src: string | null; failed: boolean } | null>(null)

  const url = a.url
  useEffect(() => {
    if (direct || !enabled) return
    const ctrl = new AbortController()
    let objectUrl: string | null = null
    fetchAttachment({ url }, ctrl.signal)
      .then((blob) => {
        objectUrl = URL.createObjectURL(blob)
        setLoaded({ url, src: objectUrl, failed: false })
      })
      .catch(() => {
        if (!ctrl.signal.aborted) setLoaded({ url, src: null, failed: true })
      })
    return () => {
      ctrl.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [url, direct, enabled])

  if (direct) return { src: direct, failed: false }
  if (loaded?.url !== a.url) return { src: null, failed: false }
  return { src: loaded.src, failed: loaded.failed }
}

/**
 * Wird das Element (bald) sichtbar? Einmal `true`, bleibt es `true` (geladene Bilder nicht wieder
 * verwerfen). Mit `active = false` sofort `true` – dann braucht es keinen Observer.
 */
function useNearViewport(ref: RefObject<Element | null>, active: boolean): boolean {
  const [seen, setSeen] = useState(() => !active || typeof IntersectionObserver === 'undefined')
  useEffect(() => {
    const el = ref.current
    if (seen || !el) return
    // Etwas Vorlauf, damit Bilder beim Scrollen schon geladen sind, wenn sie ins Bild kommen.
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setSeen(true)
      },
      { rootMargin: '400px 0px' },
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [ref, seen])
  return seen
}

/** Anzeigegröße, die das Seitenverhältnis hält und in die Vorschau-Box passt. */
function previewSize(a: Attachment): { width: number; height: number } {
  const w = a.width ?? PREVIEW_MAX_W
  const h = a.height ?? PREVIEW_MAX_H
  const scale = Math.min(1, PREVIEW_MAX_W / w, PREVIEW_MAX_H / h)
  return { width: Math.max(1, Math.round(w * scale)), height: Math.max(1, Math.round(h * scale)) }
}

export function AttachmentList({ attachments }: { attachments: Attachment[] | undefined }) {
  const [open, setOpen] = useState<Attachment | null>(null)
  if (!attachments?.length) return null
  return (
    <div className="attachments">
      {attachments.map((a) =>
        isImage(a) ? (
          <ImagePreview key={a.id} attachment={a} onOpen={() => setOpen(a)} />
        ) : (
          <FileCard key={a.id} attachment={a} />
        ),
      )}
      {open && <Lightbox attachment={open} onClose={() => setOpen(null)} />}
    </div>
  )
}

function ImagePreview({ attachment, onOpen }: { attachment: Attachment; onOpen: () => void }) {
  // Desktop-Bundle: Bilder werden per fetch (Bearer) geladen – `loading="lazy"` greift da nicht. Erst laden,
  // wenn die Vorschau in die Nähe des sichtbaren Bereichs kommt, sonst holt ein langer Verlauf alle Bilder.
  const ref = useRef<HTMLButtonElement>(null)
  const visible = useNearViewport(ref, attachmentsNeedToken())
  const { src, failed } = useAttachmentSrc(attachment, visible)
  const size = previewSize(attachment)
  return (
    <button
      ref={ref}
      type="button"
      className="attachment-image"
      data-attachment-id={attachment.id}
      data-attachment-kind="image"
      style={{ width: size.width, aspectRatio: `${size.width} / ${size.height}` }}
      onClick={onOpen}
      title={attachment.filename}
      aria-label={`Bild ${attachment.filename} vergrößern`}
    >
      {src && <img src={src} alt={attachment.filename} loading="lazy" decoding="async" />}
      {failed && <span className="muted small">Bild nicht verfügbar</span>}
    </button>
  )
}

function FileCard({ attachment }: { attachment: Attachment }) {
  return (
    <div className="attachment-file" data-attachment-id={attachment.id} data-attachment-kind="file">
      <span className="attachment-file-icon" aria-hidden="true">
        <FileText size={24} />
      </span>
      <div className="attachment-file-info">
        <span className="attachment-file-name truncate" title={attachment.filename}>
          {attachment.filename}
        </span>
        <span className="muted small">{formatBytes(attachment.size_bytes)}</span>
      </div>
      <DownloadButton attachment={attachment} />
    </div>
  )
}

function DownloadButton({ attachment, label }: { attachment: Attachment; label?: string }) {
  const [error, setError] = useState<string | null>(null)

  if (!attachmentsNeedToken()) {
    return (
      <a className="icon-button" href={attachmentUrl(attachment)} download={attachment.filename} title="Herunterladen">
        {label ?? <Download size={18} />}
      </a>
    )
  }

  async function download() {
    try {
      await downloadAttachment(attachment)
      setError(null)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <button type="button" className="icon-button" onClick={download} title={error ?? 'Herunterladen'}>
      {error ? <TriangleAlert size={18} /> : (label ?? <Download size={18} />)}
    </button>
  )
}

/** Großansicht eines Bildes (Klick daneben oder Escape schließt). */
export function Lightbox({ attachment, onClose }: { attachment: Attachment; onClose: () => void }) {
  const { src, failed } = useAttachmentSrc(attachment)
  useEscapeLayer(true, onClose)

  return (
    <div
      className="lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={attachment.filename}
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="lightbox-bar">
        <span className="truncate">{attachment.filename}</span>
        <span className="muted small">{formatBytes(attachment.size_bytes)}</span>
        <DownloadButton attachment={attachment} />
        <button type="button" className="icon-button" onClick={onClose} aria-label="Schließen">
          <X size={18} />
        </button>
      </div>
      {src && <img src={src} alt={attachment.filename} onMouseDown={(e) => e.stopPropagation()} />}
      {failed && <p className="error">Bild konnte nicht geladen werden.</p>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Composer: Auswahl, Einfügen, Drag & Drop, Fortschritt
// ---------------------------------------------------------------------------

/** Büroklammer-Button mit verstecktem Dateiauswahl-Feld. */
export function AttachButton({ channelId, disabled }: { channelId: string; disabled?: boolean }) {
  const addFiles = useAddFiles(channelId)
  const input = useRef<HTMLInputElement>(null)
  return (
    <>
      <button
        type="button"
        className="icon-button composer-attach"
        title="Datei anhängen"
        aria-label="Datei anhängen"
        disabled={disabled}
        onClick={() => input.current?.click()}
      >
        <Paperclip size={18} />
      </button>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          addFiles(Array.from(e.target.files ?? []))
          e.target.value = ''
        }}
      />
    </>
  )
}

/** `target` ist die Anzeige des Ziels, z. B. `#allgemein` oder der Name einer DM. */
export function DropOverlay({ target }: { target: string }) {
  return (
    <div className="drop-overlay" aria-hidden="true">
      <div className="drop-overlay-box">
        <strong>Dateien hier ablegen</strong>
        <span className="muted small">zum Hochladen nach {target}</span>
      </div>
    </div>
  )
}

/** Liste der Anhänge im Composer mit Fortschritt und Entfernen-Button. */
export function PendingUploads({ channelId }: { channelId: string }) {
  const uploads = useUploads((s) => s.byChannel[channelId])
  const notice = useUploads((s) => s.notice[channelId])
  const remove = useUploads((s) => s.remove)
  const dismiss = useUploads((s) => s.dismissNotice)
  const maxBytes = useStore((s) => s.instance?.max_upload_bytes)

  if (!uploads?.length && !notice) return null
  return (
    <div className="pending-uploads">
      {notice && (
        <p className="error small">
          {notice}{' '}
          <button type="button" className="link-button" onClick={() => dismiss(channelId)}>
            OK
          </button>
        </p>
      )}
      {!!uploads?.length && (
        <ul>
          {uploads.map((u) => (
            <li key={u.localId} className={`pending-upload ${u.status}`}>
              {u.previewUrl ? (
                <img src={u.previewUrl} alt="" className="pending-upload-thumb" />
              ) : (
                <span className="pending-upload-thumb" aria-hidden="true">
                  <FileText size={22} />
                </span>
              )}
              <div className="pending-upload-info">
                <span className="truncate" title={u.name}>
                  {u.name}
                </span>
                <span className="muted small">
                  {u.status === 'error'
                    ? u.error
                    : u.status === 'uploading'
                      ? `${Math.round(u.progress * 100)} % von ${formatBytes(u.size)}`
                      : formatBytes(u.size)}
                </span>
                {u.status === 'uploading' && (
                  <progress max={1} value={u.progress} aria-label={`Upload ${u.name}`} />
                )}
              </div>
              <button
                type="button"
                className="icon-button small"
                onClick={() => remove(channelId, u.localId)}
                title="Entfernen"
                aria-label={`${u.name} entfernen`}
              >
                <X size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {maxBytes !== undefined && !!uploads?.length && (
        <p className="muted small">Max. {formatBytes(maxBytes)} pro Datei, bis zu 10 Dateien.</p>
      )}
    </div>
  )
}
