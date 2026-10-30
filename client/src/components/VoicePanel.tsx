import { useEffect, useRef, useState } from 'react'
import { Video, VideoOff } from 'lucide-react'
import { useVoice } from '../lib/voice'
import type { VoiceQuality } from '../lib/voice'
import { SCREEN_QUALITIES } from '../lib/voiceSettings'
import type { ScreenQuality } from '../lib/voiceSettings'
import { dmTitle, useStore } from '../store'
import { CallPill } from './CallPill'
import { HeadphoneOff, Headphones, Mic, MicOff, MonitorOff, MonitorUp, PhoneOff, X } from 'lucide-react'

const QUALITY: Record<VoiceQuality, { label: string; level: number }> = {
  excellent: { label: 'sehr gut', level: 3 },
  good: { label: 'gut', level: 2 },
  poor: { label: 'schlecht', level: 1 },
  lost: { label: 'unterbrochen', level: 0 },
  unknown: { label: 'wird ermittelt', level: -1 },
}

/** Leiste unten in der Kanalliste, solange man in einem Voice-Kanal ist. */
export function VoicePanel() {
  const voice = useVoice()
  const channel = useStore((s) =>
    voice.communityId ? s.details[voice.communityId]?.channels.find((c) => c.id === voice.channelId) : undefined,
  )
  const community = useStore((s) => (voice.communityId ? s.details[voice.communityId]?.community : undefined))
  // Anruf in einer DM: Name der Unterhaltung statt Kanal / Community.
  const dmName = useStore((s) =>
    voice.communityId === null && voice.channelId && s.dms[voice.channelId]
      ? dmTitle(s.dms[voice.channelId], s.user?.id)
      : null,
  )
  const [menu, setMenu] = useState(false)
  // Die Vorschau eines Sprachkanals zeigt den Fehler schon selbst – nicht doppelt anzeigen.
  const previewOpen = useStore(
    (s) =>
      !s.dmView &&
      !!s.selectedCommunityId &&
      !!s.details[s.selectedCommunityId]?.channels.some((c) => c.id === s.selectedChannelId && c.kind === 'voice'),
  )

  if (voice.status === 'idle') {
    if (!voice.error || previewOpen) return null
    return (
      <div className="voice-panel voice-panel-error" role="status">
        <span className="error small">{voice.error}</span>
        <button type="button" className="icon-button small" onClick={voice.clearError} aria-label="Hinweis schließen">
          <X size={14} />
        </button>
      </div>
    )
  }

  const q = QUALITY[voice.quality]
  const statusText =
    voice.status === 'connecting'
      ? 'Verbinde …'
      : voice.status === 'reconnecting'
        ? 'Verbindet neu …'
        : 'Verbunden'
  const micOff = voice.selfMute || voice.selfDeaf || !voice.canSpeak
  const ptt = voice.settings.inputMode === 'ptt'

  return (
    <div className="voice-panel">
      <CallPill />
      <div className="voice-panel-info">
        <div className={`voice-panel-status${voice.status === 'connected' ? ' ok' : ''}`}>
          {voice.status === 'connected' && (
            <span className={`signal signal-${q.level}`} title={`Verbindung: ${q.label}`} aria-label={`Verbindung: ${q.label}`}>
              <i />
              <i />
              <i />
            </span>
          )}
          <span className="truncate">{statusText}</span>
        </div>
        <div className="truncate muted small">
          {dmName !== null ? `Anruf · ${dmName}` : (channel?.name ?? 'Sprachkanal')}
          {community ? ` / ${community.name}` : ''}
          {ptt && voice.status === 'connected' && !micOff && (
            <span className={`ptt-hint${voice.pttPressed ? ' active' : ''}`}> · PTT {voice.settings.pttKeyLabel}</span>
          )}
        </div>
        {voice.error && (
          <div className="error small voice-panel-inline-error" onClick={voice.clearError} role="status">
            {voice.error}
          </div>
        )}
      </div>
      <div className="voice-panel-actions">
        <button
          type="button"
          className={`icon-button${micOff ? ' off' : ''}`}
          onClick={voice.toggleMute}
          disabled={!voice.canSpeak}
          title={!voice.canSpeak ? 'Du darfst hier nicht sprechen' : micOff ? 'Stummschaltung aufheben' : 'Stumm schalten'}
          aria-pressed={micOff}
        >
          {micOff ? <MicOff size={18} /> : <Mic size={18} />}
        </button>
        <button
          type="button"
          className={`icon-button${voice.selfDeaf ? ' off' : ''}`}
          onClick={voice.toggleDeaf}
          title={voice.selfDeaf ? 'Taubschaltung aufheben' : 'Taub schalten'}
          aria-pressed={voice.selfDeaf}
        >
          {voice.selfDeaf ? <HeadphoneOff size={18} /> : <Headphones size={18} />}
        </button>
        <button
          type="button"
          className={`icon-button${voice.cameraOn ? ' live' : ''}`}
          disabled={(!voice.canVideo && !voice.cameraOn) || voice.status !== 'connected'}
          onClick={() => void voice.toggleCamera()}
          title={
            !voice.canVideo && !voice.cameraOn
              ? 'Du darfst hier keine Kamera nutzen'
              : voice.cameraOn
                ? 'Kamera ausschalten'
                : 'Kamera einschalten'
          }
          aria-label={voice.cameraOn ? 'Kamera ausschalten' : 'Kamera einschalten'}
          aria-pressed={voice.cameraOn}
        >
          {voice.cameraOn ? <Video size={18} aria-hidden="true" /> : <VideoOff size={18} aria-hidden="true" />}
        </button>
        <div className="voice-share">
          <button
            type="button"
            className={`icon-button${voice.screenSharing ? ' live' : ''}`}
            disabled={!voice.canStream || voice.status !== 'connected'}
            onClick={() => (voice.screenSharing ? void voice.stopScreenShare() : setMenu((m) => !m))}
            title={
              !voice.canStream
                ? 'Du darfst hier nicht streamen'
                : voice.screenSharing
                  ? 'Übertragung beenden'
                  : 'Bildschirm teilen'
            }
            aria-pressed={voice.screenSharing}
          >
            {voice.screenSharing ? <MonitorOff size={18} /> : <MonitorUp size={18} />}
          </button>
          {menu && <ShareMenu onClose={() => setMenu(false)} />}
        </div>
        <button
          type="button"
          className="icon-button hangup"
          onClick={() => void voice.leave()}
          title="Auflegen"
          aria-label="Auflegen"
        >
          <PhoneOff size={18} />
        </button>
      </div>
    </div>
  )
}

/** Qualitätswahl für die Bildschirmübertragung (Voice-Leiste und Steuerleiste der Bühne). */
export function ShareMenu({ onClose }: { onClose: () => void }) {
  const current = useVoice((s) => s.settings.screenQuality)
  const start = useVoice((s) => s.startScreenShare)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  function pick(q: ScreenQuality) {
    onClose()
    void start(q)
  }

  return (
    <div className="share-menu" ref={ref} role="menu">
      <div className="share-menu-title">Bildschirm teilen</div>
      {(Object.keys(SCREEN_QUALITIES) as ScreenQuality[]).map((q) => (
        <button
          key={q}
          type="button"
          role="menuitem"
          className={q === current ? 'active' : ''}
          onClick={() => pick(q)}
        >
          {SCREEN_QUALITIES[q].label}
        </button>
      ))}
    </div>
  )
}
