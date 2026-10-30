import { useState } from 'react'
import { Megaphone, X } from 'lucide-react'
import { useStore } from '../../store'

const DISMISSED_KEY = 'lykos_announcement_dismissed'

function readDismissed(): string | null {
  try {
    return localStorage.getItem(DISMISSED_KEY)
  } catch {
    return null
  }
}

/**
 * Ankündigung der Instanz-Admins oben in der App (`instance.announcement`, live über `INSTANCE_UPDATE`).
 * Ausblenden gilt nur für genau diesen Text – eine neue Ankündigung erscheint wieder.
 */
export function AnnouncementBanner() {
  const text = useStore((s) => s.instance?.announcement ?? null)
  const [dismissed, setDismissed] = useState(readDismissed)

  if (!text || dismissed === text) return null

  function dismiss() {
    setDismissed(text)
    try {
      if (text) localStorage.setItem(DISMISSED_KEY, text)
    } catch {
      // egal – dann nur bis zum Neuladen ausgeblendet
    }
  }

  return (
    <div className="ia-announcement" role="status">
      <Megaphone size={16} aria-hidden="true" />
      <p>{text}</p>
      <button type="button" className="icon-button small" onClick={dismiss} aria-label="Ankündigung ausblenden">
        <X size={16} />
      </button>
    </div>
  )
}
