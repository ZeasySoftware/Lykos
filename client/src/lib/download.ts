import { attachmentUrl, attachmentsNeedToken, fetchAttachment } from './api'
import type { Attachment } from './api'

/** Datei über einen temporären Link speichern. */
function saveVia(href: string, filename: string) {
  const link = document.createElement('a')
  link.href = href
  link.download = filename
  link.rel = 'noopener'
  document.body.appendChild(link)
  link.click()
  link.remove()
}

/**
 * Anhang herunterladen. Web: direkter Link (Cookie). Desktop-Bundle: per `fetch` mit Token laden und
 * als Object-URL speichern.
 */
export async function downloadAttachment(a: Pick<Attachment, 'url' | 'filename'>): Promise<void> {
  if (!attachmentsNeedToken()) {
    saveVia(attachmentUrl(a), a.filename)
    return
  }
  const blob = await fetchAttachment(a)
  const url = URL.createObjectURL(blob)
  saveVia(url, a.filename)
  setTimeout(() => URL.revokeObjectURL(url), 60_000)
}

/** Absolute Adresse eines Anhangs (zum Kopieren; Abruf braucht weiterhin eine Anmeldung). */
export function absoluteAttachmentUrl(a: Pick<Attachment, 'url'>): string {
  return new URL(attachmentUrl(a), location.href).href
}
