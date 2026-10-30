import { useState } from 'react'
import type { KeyboardEvent, ReactNode } from 'react'

/** Spoiler (`||…||`): verdeckt, bis man ihn anklickt bzw. mit Enter/Leertaste aufdeckt. */
export function Spoiler({ children }: { children: ReactNode }) {
  const [shown, setShown] = useState(false)
  const reveal = () => setShown(true)
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      reveal()
    }
  }
  return shown ? (
    <span className="spoiler shown">{children}</span>
  ) : (
    <span className="spoiler" role="button" tabIndex={0} title="Spoiler anzeigen" onClick={reveal} onKeyDown={onKey}>
      <span aria-hidden="true">{children}</span>
    </span>
  )
}
