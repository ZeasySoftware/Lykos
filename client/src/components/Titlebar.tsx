import { useEffect, useState } from 'react'
import { Copy, Minus, Square, X } from 'lucide-react'
import { titlebarContext } from '../lib/desktopShell'
import { snapHoverEnd, snapHoverStart, watchMaximized, windowControls } from '../lib/desktop'
import { dmTitle, useStore } from '../store'
import { LykosMark } from './Brand'

/**
 * Eigene Titelleiste der Desktop-App (Fenster ohne Rahmen): Wolfslogo + „Lykos“, aktueller Ort, Platz für
 * Hinweise und rechts die Fensterknöpfe im Windows-Stil. Ziehen über `data-tauri-drag-region`, Doppelklick
 * maximiert; Größe am Rand ändern, Schatten und runde Ecken (Windows 11) kommen vom Fenster selbst.
 */
export function Titlebar() {
  const context = useStore((s) => {
    if (!s.user) return null
    const me = s.user.id
    const dm = s.dmView && s.selectedDmId ? s.dms[s.selectedDmId] : undefined
    const community = s.communities.find((c) => c.id === s.selectedCommunityId)
    const detail = s.selectedCommunityId ? s.details[s.selectedCommunityId] : undefined
    const channel = detail?.channels.find((c) => c.id === s.selectedChannelId)
    return titlebarContext({
      dmView: s.dmView,
      dmName: dm ? dmTitle(dm, me) : null,
      communityName: community?.name ?? null,
      channel: channel ? { name: channel.name, kind: channel.kind } : null,
    })
  })
  const [maximized, setMaximized] = useState(false)
  useEffect(() => watchMaximized(setMaximized), [])

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <LykosMark className="titlebar-mark" />
        <span className="titlebar-name">Lykos</span>
      </div>
      {context && (
        <div className="titlebar-context truncate" title={context}>
          {context}
        </div>
      )}
      <div className="titlebar-spacer" data-tauri-drag-region />
      {/* Platz für Hinweise (z. B. Update-Pfeil). */}
      <div className="titlebar-actions" />
      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-button"
          aria-label="Minimieren"
          title="Minimieren"
          tabIndex={-1}
          onClick={windowControls.minimize}
        >
          <Minus size={16} strokeWidth={1.5} />
        </button>
        <button
          type="button"
          className="titlebar-button"
          aria-label={maximized ? 'Wiederherstellen' : 'Maximieren'}
          tabIndex={-1}
          onClick={() => {
            snapHoverEnd()
            windowControls.toggleMaximize()
          }}
          onMouseEnter={snapHoverStart}
          onMouseLeave={snapHoverEnd}
        >
          {maximized ? (
            <Copy size={13} strokeWidth={1.5} className="titlebar-restore" />
          ) : (
            <Square size={12} strokeWidth={1.5} />
          )}
        </button>
        <button
          type="button"
          className="titlebar-button titlebar-close"
          aria-label="Schließen"
          title="Schließen"
          tabIndex={-1}
          onClick={windowControls.close}
        >
          <X size={17} strokeWidth={1.5} />
        </button>
      </div>
    </header>
  )
}
