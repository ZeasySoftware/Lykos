import { useEffect, useRef } from 'react'
import type { CSSProperties, KeyboardEvent, MouseEvent } from 'react'
import type { Track } from 'livekit-client'
import { Eye, HeadphoneOff, Maximize, MicOff, Monitor, WifiOff, X } from 'lucide-react'
import { avatarColor } from '../lib/avatar'
import { buildCallTileMenu } from '../lib/contextMenus'
import { useContextMenu } from '../lib/menuStore'
import { toggleFullscreen } from '../lib/call'
import type { CallMember } from '../lib/call'
import type { CallTile } from '../lib/callLayout'
import { cameraTrack, requestCamera, screenTrack, useVoice } from '../lib/voice'
import type { VoiceQuality } from '../lib/voice'
import { Avatar } from './Presence'

/** `grid` = Raster, `spotlight` = fokussiert (groß), `strip` = Leiste im Fokus-Modus, `pill` = Mini-Ansicht. */
export type TileVariant = 'grid' | 'spotlight' | 'strip' | 'pill'

const AVATAR_SIZE: Record<TileVariant, number> = { grid: 80, spotlight: 112, strip: 40, pill: 36 }

/**
 * Kamera-Spur für eine sichtbare Kachel: fordert das Abo an, solange die Kachel gemountet ist
 * (adaptiveStream wählt dann die Simulcast-Ebene nach der Elementgröße), und gibt es beim Unmount frei.
 */
function useCameraTrack(userId: string, enabled: boolean): Track | undefined {
  useVoice((s) => s.trackVersion) // neu rendern, sobald Spuren kommen/gehen
  useEffect(() => (enabled ? requestCamera(userId) : undefined), [userId, enabled])
  return enabled ? cameraTrack(userId) : undefined
}

function useScreenTrack(userId: string, enabled: boolean): Track | undefined {
  useVoice((s) => s.trackVersion)
  return enabled ? screenTrack(userId) : undefined
}

/** Hängt eine Spur an ein `<video>` und beim Unmount/Wechsel wieder ab. */
export function VideoSurface({ track, mirrored, contain }: { track: Track | undefined; mirrored?: boolean; contain?: boolean }) {
  const ref = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    const el = ref.current
    if (!track || !el) return
    track.attach(el)
    return () => {
      track.detach(el)
    }
  }, [track])
  if (!track) return null
  // Stumm: Ton kommt über die eigenen Audio-Spuren (bzw. die eigene Vorschau soll kein Echo erzeugen).
  return (
    <video
      ref={ref}
      className={`call-video${mirrored ? ' mirrored' : ''}${contain ? ' contain' : ''}`}
      autoPlay
      playsInline
      muted
    />
  )
}

/** Hintergrund einer Kachel ohne Bild: Profilfarbe gedimmt, Avatar mittig. */
function AvatarBackdrop({ member, variant, speaking }: { member: CallMember; variant: TileVariant; speaking: boolean }) {
  return (
    <div
      className="call-backdrop"
      style={{ '--tile-color': avatarColor(member.user?.id ?? member.userId) } as CSSProperties}
    >
      <span className={`call-avatar${speaking ? ' speaking' : ''}`}>
        <Avatar user={member.user ?? { id: member.userId }} name={member.name} size={AVATAR_SIZE[variant]} />
      </span>
    </div>
  )
}

export function CallTileView({
  tile,
  member,
  variant,
  onActivate,
  style,
}: {
  tile: CallTile
  member: CallMember
  variant: TileVariant
  /** Klick auf die Kachel (Fokus umschalten bzw. Mini-Ansicht → Bühne). */
  onActivate?: () => void
  style?: CSSProperties
}) {
  const speaking = useVoice((s) => tile.kind === 'user' && s.speaking.includes(tile.userId))
  const quality = useVoice((s): VoiceQuality => (member.isMe ? s.quality : (s.qualities[tile.userId] ?? 'unknown')))
  const watch = useVoice((s) => s.watch)
  const unwatch = useVoice((s) => s.unwatch)
  const setFocus = useVoice((s) => s.setFocus)
  const ref = useRef<HTMLDivElement>(null)
  const menu = useContextMenu(
    (ctx) =>
      buildCallTileMenu({ tile, isMe: member.isMe, target: ctx.target, onFullscreen: () => toggleFullscreen(ref.current) }),
    `Kachel ${member.name}`,
  )

  const cam = useCameraTrack(tile.userId, tile.kind === 'user' && tile.camera)
  const screen = useScreenTrack(tile.userId, tile.kind === 'screen' && tile.watching)
  const track = tile.kind === 'user' ? cam : screen
  const small = variant === 'strip' || variant === 'pill'
  const badLink = quality === 'poor' || quality === 'lost'

  function onKey(e: KeyboardEvent) {
    if (e.target !== e.currentTarget || !onActivate) return
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onActivate()
    }
  }
  const stop = (fn: () => void) => (e: MouseEvent) => {
    e.stopPropagation()
    fn()
  }
  function fullscreen(e?: MouseEvent) {
    e?.stopPropagation()
    toggleFullscreen(ref.current)
  }

  const label =
    tile.kind === 'screen'
      ? member.isMe
        ? 'Deine Übertragung'
        : `Bildschirm von ${member.name}`
      : member.isMe
        ? `${member.name} (du)`
        : member.name

  return (
    <div
      ref={ref}
      className={`call-tile ${variant} ${tile.kind}${speaking ? ' speaking' : ''}${track ? ' has-video' : ''}`}
      style={style}
      role={onActivate ? 'button' : undefined}
      tabIndex={onActivate ? 0 : undefined}
      aria-label={label}
      onClick={onActivate}
      onKeyDown={(e) => {
        menu.onKeyDown(e)
        onKey(e)
      }}
      onContextMenu={menu.onContextMenu}
      data-menu-anchor
      onDoubleClick={variant === 'pill' ? undefined : fullscreen}
    >
      {tile.kind === 'screen' && !tile.watching ? (
        <div className="call-stream-offer" style={{ '--tile-color': avatarColor(tile.userId) } as CSSProperties}>
          <Monitor size={small ? 18 : 28} aria-hidden="true" />
          {!small && <span className="call-stream-offer-text">{member.name} überträgt den Bildschirm</span>}
          <button
            type="button"
            className="button primary call-watch"
            onClick={stop(() => {
              watch(tile.userId)
              setFocus(tile.key)
            })}
          >
            <Eye size={16} aria-hidden="true" />
            {small ? 'Ansehen' : 'Stream ansehen'}
          </button>
        </div>
      ) : (
        <>
          {tile.kind === 'user' && <AvatarBackdrop member={member} variant={variant} speaking={speaking && !track} />}
          {tile.kind === 'screen' && !track && <div className="call-wait">Stream wird geladen …</div>}
          <VideoSurface
            track={track}
            mirrored={tile.kind === 'user' && member.isMe}
            contain={tile.kind === 'screen' || variant === 'spotlight'}
          />
        </>
      )}

      <div className="call-tile-label">
        {tile.kind === 'screen' && <span className="live-badge">LIVE</span>}
        <span className="truncate">{label}</span>
        {tile.kind === 'user' &&
          (member.deaf ? (
            <span className="call-flag" title="Taub geschaltet" aria-label="Taub geschaltet">
              <HeadphoneOff size={14} />
            </span>
          ) : (
            member.mute && (
              <span className="call-flag" title="Stumm geschaltet" aria-label="Stumm geschaltet">
                <MicOff size={14} />
              </span>
            )
          ))}
      </div>

      {tile.kind === 'user' && badLink && !small && (
        <span
          className={`call-quality ${quality}`}
          title={quality === 'lost' ? 'Verbindung unterbrochen' : 'Schlechte Verbindung'}
          aria-label={quality === 'lost' ? 'Verbindung unterbrochen' : 'Schlechte Verbindung'}
        >
          <WifiOff size={14} />
        </span>
      )}

      {variant !== 'pill' && variant !== 'strip' && (
        <div className="call-tile-actions">
          {tile.kind === 'screen' && tile.watching && !member.isMe && (
            <button
              type="button"
              className="call-tile-button"
              onClick={stop(() => unwatch(tile.userId))}
              title="Zuschauen beenden"
              aria-label="Zuschauen beenden"
            >
              <X size={16} />
            </button>
          )}
          <button
            type="button"
            className="call-tile-button"
            onClick={fullscreen}
            title="Vollbild"
            aria-label="Vollbild"
          >
            <Maximize size={16} />
          </button>
        </div>
      )}
    </div>
  )
}
