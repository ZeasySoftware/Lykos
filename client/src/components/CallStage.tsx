import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, ReactNode, RefObject } from 'react'
import {
  ChevronUp,
  HeadphoneOff,
  Headphones,
  Maximize,
  Mic,
  MicOff,
  Minimize,
  MonitorOff,
  MonitorUp,
  PhoneOff,
  Video,
  VideoOff,
  Volume2,
  X,
} from 'lucide-react'
import type { Channel, CommunityDetail } from '../lib/api'
import { toggleFullscreen, useCallMembers } from '../lib/call'
import type { CallMember } from '../lib/call'
import { MAX_GRID_TILES, buildTiles, gridLayout, hasVideo, pickVisible } from '../lib/callLayout'
import type { CallTile } from '../lib/callLayout'
import { Permission, has } from '../lib/permissions'
import { CALL_SHORTCUTS, shortcutLabel, useVoice } from '../lib/voice'
import { dmTitle, useStore } from '../store'
import { CallTileView } from './CallTile'
import { DrawerButton } from './MobileNav'
import { useIsMobile } from '../lib/mobile'
import { Avatar } from './Presence'
import { ShareMenu } from './VoicePanel'

/** Abstand zwischen Kacheln (px) – wie `--call-gap` in call.css. */
const GAP = 8
/** Steuerleiste blendet nach so langer Inaktivität aus, wenn Videos laufen. */
const IDLE_MS = 3000

/** Mitgliederliste für Namen nachladen, falls noch nicht da. */
function useEnsureMembers(communityId: string) {
  const hasMembers = useStore((s) => !!s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  useEffect(() => {
    if (!hasMembers) void loadMembers(communityId).catch(() => {})
  }, [hasMembers, communityId, loadMembers])
}

function useElementSize(ref: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 })
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect
      setSize((s) => (s.width === Math.round(width) && s.height === Math.round(height) ? s : { width: Math.round(width), height: Math.round(height) }))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [ref])
  return size
}

/** Raum der Bühne: Sprachkanal einer Community oder DM-Kanal (`communityId: null`, Anruf). */
export interface StageRoom {
  id: string
  communityId: string | null
  name: string
}

/** Hauptbereich eines Sprachkanals: Bühne, wenn man drin ist, sonst Vorschau mit „Beitreten“. */
export function CallStage({ channel, detail }: { channel: Channel; detail: CommunityDetail }) {
  const inRoom = useVoice((s) => s.status !== 'idle' && s.channelId === channel.id)
  useEnsureMembers(channel.community_id)
  const room = useMemo(
    () => ({ id: channel.id, communityId: channel.community_id, name: channel.name }),
    [channel.id, channel.community_id, channel.name],
  )
  return inRoom ? <CallRoom room={room} /> : <CallPreview channel={channel} detail={detail} />
}

// ---------------------------------------------------------------------------
// Vorschau (nicht beigetreten)
// ---------------------------------------------------------------------------

function CallPreview({ channel, detail }: { channel: Channel; detail: CommunityDetail }) {
  const people = useCallMembers(channel.community_id, channel.id)
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const join = useVoice((s) => s.join)
  const error = useVoice((s) => s.error)
  const clearError = useVoice((s) => s.clearError)
  const inCall = useVoice((s) => s.status !== 'idle')
  const callCommunity = useVoice((s) => s.communityId)
  const callChannel = useVoice((s) => s.channelId)
  const otherCall = useStore((s) =>
    !inCall || !callChannel
      ? null
      : callCommunity
        ? (s.details[callCommunity]?.channels.find((c) => c.id === callChannel)?.name ?? null)
        : s.dms[callChannel]
          ? `Anruf mit ${dmTitle(s.dms[callChannel], s.user?.id)}`
          : null,
  )
  const perms = detail.channel_permissions[channel.id] ?? detail.permissions
  const canConnect = has(perms, Permission.CONNECT)
  const shown = people.slice(0, 5)
  const more = people.length - shown.length

  // „Anna“, „Anna und Ben“, „Anna, Ben und Carla“, „Anna, Ben, Carla und 2 weitere“
  const names = people.map((p) => p.name)
  const listed = names.length > 3 ? [...names.slice(0, 3), `${names.length - 3} weitere`] : names
  const nameList = listed.length > 1 ? `${listed.slice(0, -1).join(', ')} und ${listed.at(-1)}` : (listed[0] ?? '')

  return (
    <div className="call-stage">
      <StageHeader name={channel.name} count={people.length} />
      <div className="call-preview">
        <div className="call-preview-avatars" aria-hidden="true">
          {shown.length === 0 ? (
            <span className="call-preview-empty">
              <Volume2 size={36} />
            </span>
          ) : (
            shown.map((p) => (
              <span key={p.userId} className="call-preview-avatar">
                <Avatar user={p.user ?? { id: p.userId }} name={p.name} size={64} />
              </span>
            ))
          )}
          {more > 0 && <span className="call-preview-more">+{more}</span>}
        </div>
        <h2>{channel.name}</h2>
        <p className="call-preview-count">
          {people.length === 0
            ? 'Noch niemand ist hier.'
            : `${people.length === 1 ? '1 Person' : `${people.length} Leute`} im Kanal`}
        </p>
        {nameList && <p className="muted">{nameList}</p>}
        <button
          type="button"
          className="button primary call-join"
          disabled={!voiceEnabled || !canConnect}
          onClick={() => void join(channel)}
        >
          Beitreten
        </button>
        {!voiceEnabled ? (
          <p className="muted small">Sprachkanäle sind auf dieser Instanz nicht eingerichtet.</p>
        ) : !canConnect ? (
          <p className="muted small">Dir fehlt die Berechtigung, diesem Sprachkanal beizutreten.</p>
        ) : (
          otherCall && <p className="muted small">Du bist gerade in „{otherCall}“ – Beitreten wechselt den Kanal.</p>
        )}
        {error && (
          <p className="error small call-preview-error" role="status">
            {error}
            <button type="button" className="icon-button small" onClick={clearError} aria-label="Hinweis schließen">
              <X size={14} />
            </button>
          </p>
        )}
      </div>
    </div>
  )
}

function StageHeader({ name, count, children }: { name: string; count: number; children?: ReactNode }) {
  return (
    <header className="pane-header call-header">
      <DrawerButton />
      <span className="channel-icon" aria-hidden="true">
        <Volume2 size={20} />
      </span>
      <strong className="truncate">{name}</strong>
      <span className="pane-topic">{count === 1 ? '1 Teilnehmer' : `${count} Teilnehmer`}</span>
      {children}
    </header>
  )
}

// ---------------------------------------------------------------------------
// Bühne (beigetreten)
// ---------------------------------------------------------------------------

/**
 * Bühne des eigenen Calls. `embedded`: ohne eigene Kopfzeile, z. B. über dem Verlauf einer DM (deren Kopf
 * bleibt stehen); `banner` erscheint unter den Hinweisen (z. B. „Klingelt bei …“).
 */
export function CallRoom({ room, embedded = false, banner }: { room: StageRoom; embedded?: boolean; banner?: ReactNode }) {
  const people = useCallMembers(room.communityId, room.id)
  const me = useStore((s) => s.user?.id)
  const watching = useVoice((s) => s.watching)
  const speaking = useVoice((s) => s.speaking)
  const focus = useVoice((s) => s.focus)
  const setFocus = useVoice((s) => s.setFocus)
  const status = useVoice((s) => s.status)
  const error = useVoice((s) => s.error)
  const clearError = useVoice((s) => s.clearError)
  const stageRef = useRef<HTMLDivElement>(null)
  const [showAll, setShowAll] = useState(false)
  const [fullscreen, setFullscreen] = useState(false)

  const byId = useMemo(() => new Map(people.map((p) => [p.userId, p])), [people])
  const tiles = useMemo(() => buildTiles(people, watching, me), [people, watching, me])
  const focused = tiles.find((t) => t.key === focus) ?? null
  const anyVideo = tiles.some(hasVideo)

  // Steuerleiste bei Inaktivität ausblenden – nur wenn Videos laufen.
  const [idle, setIdle] = useState(false)
  const hold = useRef(false)
  const timer = useRef<number | undefined>(undefined)
  const arm = useCallback(() => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      if (!hold.current) setIdle(true)
    }, IDLE_MS)
  }, [])
  const wake = useCallback(() => {
    setIdle(false)
    arm()
  }, [arm])
  useEffect(() => {
    arm()
    return () => window.clearTimeout(timer.current)
  }, [arm])
  const setHold = useCallback(
    (on: boolean) => {
      hold.current = on
      if (!on) wake()
    },
    [wake],
  )

  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === stageRef.current)
    document.addEventListener('fullscreenchange', onChange)
    return () => document.removeEventListener('fullscreenchange', onChange)
  }, [])

  // Esc beendet den Fokus (Vollbild beendet der Browser selbst).
  useEffect(() => {
    if (!focused) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !document.fullscreenElement && !e.defaultPrevented) setFocus(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [focused, setFocus])

  const toggleFocus = (t: CallTile) => setFocus(focus === t.key ? null : t.key)

  return (
    <div
      ref={stageRef}
      className={`call-stage live${embedded ? ' embedded' : ''}${idle && anyVideo ? ' idle' : ''}`}
      onPointerMove={wake}
      onPointerDown={wake}
      onKeyDown={wake}
    >
      {!embedded && <StageHeader name={room.name} count={people.length} />}
      {error && (
        <div className="call-banner" role="status">
          <span>{error}</span>
          <button type="button" className="icon-button small" onClick={clearError} aria-label="Hinweis schließen">
            <X size={14} />
          </button>
        </div>
      )}
      {status !== 'connected' && (
        <div className="call-banner warn" role="status">
          {status === 'reconnecting' ? 'Verbindung wird wiederhergestellt …' : 'Verbinde …'}
        </div>
      )}
      {banner}
      {focused ? (
        <FocusLayout tiles={tiles} focused={focused} byId={byId} onActivate={toggleFocus} />
      ) : (
        <GridLayoutView
          tiles={tiles}
          byId={byId}
          speaking={speaking}
          showAll={showAll}
          onShowAll={setShowAll}
          onActivate={toggleFocus}
        />
      )}
      <CallControls
        fullscreen={fullscreen}
        onFullscreen={() => toggleFullscreen(stageRef.current)}
        onHold={setHold}
      />
    </div>
  )
}

function GridLayoutView({
  tiles,
  byId,
  speaking,
  showAll,
  onShowAll,
  onActivate,
}: {
  tiles: CallTile[]
  byId: Map<string, CallMember>
  speaking: string[]
  showAll: boolean
  onShowAll: (all: boolean) => void
  onActivate: (t: CallTile) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const size = useElementSize(ref)
  const speakingSet = useMemo(() => new Set(speaking), [speaking])
  // Mobil: eine Spalte, scrollbar – Kacheln bleiben so groß genug zum Erkennen.
  const mobile = useIsMobile()
  const overflow = tiles.length > MAX_GRID_TILES
  const scroll = (overflow && showAll) || mobile
  const { visible, hidden } = scroll ? { visible: tiles, hidden: [] } : pickVisible(tiles, MAX_GRID_TILES, speakingSet)
  const count = visible.length + (hidden.length > 0 ? 1 : 0)

  // Scroll-Modus: feste Mindestbreite, sonst die beste Aufteilung für die Fläche.
  let tileStyle: CSSProperties
  if (scroll) {
    const cols = mobile ? 1 : Math.max(1, Math.floor((size.width + GAP) / (260 + GAP)))
    const w = Math.floor((size.width - GAP * (cols - 1)) / cols)
    tileStyle = { width: w, height: Math.floor((w * 9) / 16) }
  } else {
    const l = gridLayout(count, size.width, size.height, GAP)
    tileStyle = { width: l.tileWidth, height: l.tileHeight }
  }

  return (
    <div ref={ref} className={`call-grid${scroll ? ' scroll' : ''}`}>
      {size.width > 0 &&
        visible.map((t) => (
          <CallTileView
            key={t.key}
            tile={t}
            member={byId.get(t.userId)!}
            variant="grid"
            style={tileStyle}
            onActivate={() => onActivate(t)}
          />
        ))}
      {size.width > 0 && hidden.length > 0 && (
        <button type="button" className="call-tile call-more" style={tileStyle} onClick={() => onShowAll(true)}>
          <span className="call-more-avatars" aria-hidden="true">
            {hidden.slice(0, 3).map((t) => {
              const m = byId.get(t.userId)!
              return <Avatar key={t.key} user={m.user ?? { id: m.userId }} name={m.name} size={36} />
            })}
          </span>
          <span>+{hidden.length} weitere</span>
        </button>
      )}
      {overflow && showAll && (
        <button type="button" className="button call-less" onClick={() => onShowAll(false)}>
          Weniger anzeigen
        </button>
      )}
    </div>
  )
}

function FocusLayout({
  tiles,
  focused,
  byId,
  onActivate,
}: {
  tiles: CallTile[]
  focused: CallTile
  byId: Map<string, CallMember>
  onActivate: (t: CallTile) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const size = useElementSize(ref)
  const others = tiles.filter((t) => t !== focused)
  const l = gridLayout(1, size.width, size.height, 0)
  return (
    <div className="call-focus">
      <div ref={ref} className="call-spotlight">
        {size.width > 0 && (
          <CallTileView
            tile={focused}
            member={byId.get(focused.userId)!}
            variant="spotlight"
            style={{ width: l.tileWidth, height: l.tileHeight }}
            onActivate={() => onActivate(focused)}
          />
        )}
      </div>
      {others.length > 0 && (
        <div className="call-strip" role="list" aria-label="Weitere Teilnehmer">
          {others.map((t) => (
            <CallTileView
              key={t.key}
              tile={t}
              member={byId.get(t.userId)!}
              variant="strip"
              onActivate={() => onActivate(t)}
            />
          ))}
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Steuerleiste
// ---------------------------------------------------------------------------

function CallControls({
  fullscreen,
  onFullscreen,
  onHold,
}: {
  fullscreen: boolean
  onFullscreen: () => void
  onHold: (hold: boolean) => void
}) {
  const voice = useVoice()
  const [menu, setMenu] = useState<'camera' | 'share' | null>(null)
  const micOff = voice.selfMute || voice.selfDeaf || !voice.canSpeak
  const connected = voice.status === 'connected'
  const cameraBlocked = !voice.canVideo && !voice.cameraOn

  useEffect(() => onHold(menu !== null), [menu, onHold])
  const close = useCallback(() => setMenu(null), [])

  return (
    <div
      className="call-controls"
      onPointerEnter={() => onHold(true)}
      onPointerLeave={() => onHold(menu !== null)}
    >
      <div className="call-controls-bar" role="toolbar" aria-label="Call-Steuerung">
        <button
          type="button"
          className={`call-control${micOff ? ' off' : ''}`}
          onClick={voice.toggleMute}
          disabled={!voice.canSpeak}
          title={
            !voice.canSpeak
              ? 'Du darfst hier nicht sprechen'
              : `${micOff ? 'Stummschaltung aufheben' : 'Stumm schalten'} (${shortcutLabel(CALL_SHORTCUTS.mute)})`
          }
          aria-label={micOff ? 'Stummschaltung aufheben' : 'Stumm schalten'}
          aria-keyshortcuts={`Control+Shift+${CALL_SHORTCUTS.mute}`}
          aria-pressed={micOff}
        >
          {micOff ? <MicOff size={20} /> : <Mic size={20} />}
        </button>
        <button
          type="button"
          className={`call-control${voice.selfDeaf ? ' off' : ''}`}
          onClick={voice.toggleDeaf}
          title={`${voice.selfDeaf ? 'Taubschaltung aufheben' : 'Taub schalten'} (${shortcutLabel(CALL_SHORTCUTS.deaf)})`}
          aria-label={voice.selfDeaf ? 'Taubschaltung aufheben' : 'Taub schalten'}
          aria-keyshortcuts={`Control+Shift+${CALL_SHORTCUTS.deaf}`}
          aria-pressed={voice.selfDeaf}
        >
          {voice.selfDeaf ? <HeadphoneOff size={20} /> : <Headphones size={20} />}
        </button>

        <div className="call-control-group">
          <button
            type="button"
            className={`call-control${voice.cameraOn ? ' on' : ''}`}
            disabled={cameraBlocked || !connected}
            onClick={() => void voice.toggleCamera()}
            title={
              cameraBlocked
                ? 'Du darfst hier keine Kamera nutzen'
                : `${voice.cameraOn ? 'Kamera ausschalten' : 'Kamera einschalten'} (${shortcutLabel(CALL_SHORTCUTS.camera)})`
            }
            aria-label={voice.cameraOn ? 'Kamera ausschalten' : 'Kamera einschalten'}
            aria-keyshortcuts={`Control+Shift+${CALL_SHORTCUTS.camera}`}
            aria-pressed={voice.cameraOn}
          >
            {voice.cameraOn ? <Video size={20} /> : <VideoOff size={20} />}
          </button>
          <button
            type="button"
            className="call-control-more"
            disabled={cameraBlocked}
            onClick={() => setMenu((m) => (m === 'camera' ? null : 'camera'))}
            title="Kamera wählen"
            aria-label="Kamera wählen"
            aria-expanded={menu === 'camera'}
          >
            <ChevronUp size={14} />
          </button>
          {menu === 'camera' && <CameraMenu onClose={close} />}
        </div>

        <div className="call-control-group">
          <button
            type="button"
            className={`call-control${voice.screenSharing ? ' on' : ''}`}
            disabled={!voice.canStream || !connected}
            onClick={() => (voice.screenSharing ? void voice.stopScreenShare() : setMenu((m) => (m === 'share' ? null : 'share')))}
            title={
              !voice.canStream ? 'Du darfst hier nicht streamen' : voice.screenSharing ? 'Übertragung beenden' : 'Bildschirm teilen'
            }
            aria-label={voice.screenSharing ? 'Übertragung beenden' : 'Bildschirm teilen'}
            aria-pressed={voice.screenSharing}
          >
            {voice.screenSharing ? <MonitorOff size={20} /> : <MonitorUp size={20} />}
          </button>
          {menu === 'share' && <ShareMenu onClose={close} />}
        </div>

        <button
          type="button"
          className="call-control hangup"
          onClick={() => void voice.leave()}
          title="Auflegen"
          aria-label="Auflegen"
        >
          <PhoneOff size={20} />
        </button>
      </div>
      <button
        type="button"
        className="call-control call-fullscreen"
        onClick={onFullscreen}
        title={fullscreen ? 'Vollbild beenden' : 'Bühne im Vollbild'}
        aria-label={fullscreen ? 'Vollbild beenden' : 'Bühne im Vollbild'}
      >
        {fullscreen ? <Minimize size={18} /> : <Maximize size={18} />}
      </button>
    </div>
  )
}

function CameraMenu({ onClose }: { onClose: () => void }) {
  const current = useVoice((s) => s.settings.videoDeviceId)
  const updateSettings = useVoice((s) => s.updateSettings)
  const [devices, setDevices] = useState<MediaDeviceInfo[] | null>(null)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let alive = true
    void navigator.mediaDevices
      ?.enumerateDevices()
      .then((list) => alive && setDevices(list.filter((d) => d.kind === 'videoinput')))
      .catch(() => alive && setDevices([]))
    return () => {
      alive = false
    }
  }, [])

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  function pick(id: string) {
    onClose()
    updateSettings({ videoDeviceId: id })
  }

  return (
    <div className="share-menu call-menu" ref={ref} role="menu">
      <div className="share-menu-title">Kamera</div>
      {devices === null && <div className="muted small call-menu-note">Wird geladen …</div>}
      {devices?.length === 0 && <div className="muted small call-menu-note">Keine Kamera gefunden.</div>}
      {devices && devices.length > 0 && (
        <>
          <button type="button" role="menuitem" className={!current ? 'active' : ''} onClick={() => pick('')}>
            Standard
          </button>
          {devices.map((d, i) => (
            <button
              key={d.deviceId || i}
              type="button"
              role="menuitem"
              className={current === d.deviceId ? 'active' : ''}
              onClick={() => pick(d.deviceId)}
            >
              <span className="truncate">{d.label || `Kamera ${i + 1}`}</span>
            </button>
          ))}
        </>
      )}
    </div>
  )
}
