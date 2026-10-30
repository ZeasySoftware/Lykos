import { useMemo, useRef } from 'react'
import type { MouseEvent } from 'react'
import { Maximize2, PictureInPicture2 } from 'lucide-react'
import { buildTiles, featuredTile } from '../lib/callLayout'
import { openStage, useCallMembers, useStageVisible } from '../lib/call'
import { useVoice } from '../lib/voice'
import { useStore } from '../store'
import { CallTileView } from './CallTile'
import { Avatar } from './Presence'

/**
 * Mini-Ansicht über der Voice-Leiste, solange man im Call ist, die Bühne aber nicht sieht
 * (z. B. in einem Textkanal): fokussierte Kachel bzw. Kamera der zuletzt sprechenden Person,
 * sonst die Avatare. Klick → zurück zur Bühne.
 */
export function CallPill() {
  const status = useVoice((s) => s.status)
  const communityId = useVoice((s) => s.communityId)
  const channelId = useVoice((s) => s.channelId)
  const stageVisible = useStageVisible()
  if (status === 'idle' || !channelId || stageVisible) return null
  return <CallPillInner communityId={communityId} channelId={channelId} />
}

/** `communityId = null`: Anruf in einer DM (Klick öffnet die DM). */
function CallPillInner({ communityId, channelId }: { communityId: string | null; channelId: string }) {
  const people = useCallMembers(communityId, channelId)
  const me = useStore((s) => s.user?.id)
  const watching = useVoice((s) => s.watching)
  const focus = useVoice((s) => s.focus)
  const lastSpeaker = useVoice((s) => s.lastSpeaker)
  const speaking = useVoice((s) => s.speaking)
  const ref = useRef<HTMLDivElement>(null)

  const tiles = useMemo(() => buildTiles(people, watching, me), [people, watching, me])
  const featured = featuredTile(tiles, focus, lastSpeaker, me)
  const member = featured ? people.find((p) => p.userId === featured.userId) : undefined
  const open = () => openStage(communityId, channelId)
  const pipSupported = typeof document !== 'undefined' && 'pictureInPictureEnabled' in document && document.pictureInPictureEnabled

  function pip(e: MouseEvent) {
    e.stopPropagation()
    const video = ref.current?.querySelector('video')
    if (!video) return
    if (document.pictureInPictureElement) void document.exitPictureInPicture().catch(() => {})
    else void video.requestPictureInPicture().catch(() => {})
  }

  if (featured && member) {
    return (
      <div className="call-pill" ref={ref}>
        <CallTileView tile={featured} member={member} variant="pill" onActivate={open} />
        {pipSupported && (
          <button
            type="button"
            className="call-pill-pip call-tile-button"
            onClick={pip}
            title="Bild im Bild"
            aria-label="Bild im Bild"
          >
            <PictureInPicture2 size={14} />
          </button>
        )}
      </div>
    )
  }

  // Ohne Bild: Avatare (sprechende mit grünem Ring) + Hinweis.
  const shown = people.slice(0, 4)
  return (
    <button type="button" className="call-pill call-pill-row" onClick={open} title="Zur Call-Ansicht">
      <span className="call-pill-avatars" aria-hidden="true">
        {shown.map((p) => (
          <span key={p.userId} className={`call-avatar${speaking.includes(p.userId) ? ' speaking' : ''}`}>
            <Avatar user={p.user ?? { id: p.userId }} name={p.name} size={24} />
          </span>
        ))}
        {people.length > shown.length && <span className="call-pill-more">+{people.length - shown.length}</span>}
      </span>
      <span className="truncate call-pill-text">Zur Call-Ansicht</span>
      <Maximize2 size={14} aria-hidden="true" />
    </button>
  )
}
