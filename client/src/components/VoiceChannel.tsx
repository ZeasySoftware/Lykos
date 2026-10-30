import { useEffect } from 'react'
import { Video } from 'lucide-react'
import type { Channel, CommunityDetail, VoiceState } from '../lib/api'
import { buildChannelMenu, buildUserMenu } from '../lib/contextMenus'
import { useContextMenu } from '../lib/menuStore'
import { isChannelMuted, useMutesVersion } from '../lib/notify'
import { Permission, has } from '../lib/permissions'
import { openStage } from '../lib/call'
import { tileKey } from '../lib/callLayout'
import { useVoice } from '../lib/voice'
import { displayName, memberName, useMember, useStore } from '../store'
import { Avatar } from './Presence'
import { HeadphoneOff, MicOff, Settings, Volume2, VolumeX } from 'lucide-react'

const EMPTY: VoiceState[] = []
/** Communities, deren Mitgliederliste gerade für Voice-Namen geladen wird. */
const loadingMembers = new Set<string>()

/** Voice-Kanal in der Kanalliste: Klick = beitreten, darunter die Teilnehmer. */
export function VoiceChannel({
  channel,
  detail,
  onSettings,
}: {
  channel: Channel
  detail: CommunityDetail
  /** Kanal-Einstellungen öffnen (nur mit MANAGE_CHANNELS/MANAGE_ROLES gesetzt). */
  onSettings?: () => void
}) {
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const states = useStore((s) => s.voiceStates[channel.community_id] ?? EMPTY)
  const join = useVoice((s) => s.join)
  const idle = useVoice((s) => s.status === 'idle')
  const connectedChannel = useVoice((s) => (s.status === 'idle' ? null : s.channelId))
  const selected = useStore((s) => s.selectedChannelId === channel.id)

  const menu = useContextMenu(() => buildChannelMenu(channel), `Sprachkanal ${channel.name}`)
  useMutesVersion()
  const muted = isChannelMuted(channel.id)
  const perms = detail.channel_permissions[channel.id] ?? detail.permissions
  const canConnect = has(perms, Permission.CONNECT)
  const participants = states.filter((v) => v.channel_id === channel.id)
  const connected = connectedChannel === channel.id

  // Namen der Teilnehmer kommen aus der Mitgliederliste – ggf. nachladen.
  const hasMembers = useStore((s) => !!s.members[channel.community_id])
  const loadMembers = useStore((s) => s.loadMembers)
  const needsNames = participants.length > 0 && !hasMembers
  useEffect(() => {
    const id = channel.community_id
    if (!needsNames || loadingMembers.has(id)) return
    loadingMembers.add(id)
    loadMembers(id)
      .catch(() => {})
      .finally(() => loadingMembers.delete(id))
  }, [needsNames, channel.community_id, loadMembers])

  let title = channel.topic ?? channel.name
  if (!voiceEnabled) title = 'Sprachkanäle sind auf dieser Instanz nicht eingerichtet.'
  else if (!canConnect) title = 'Dir fehlt die Berechtigung, diesem Sprachkanal beizutreten.'

  // Wie bei Discord: Klick zeigt die Call-Bühne und tritt bei, solange man in keinem anderen Call ist
  // (aus einem laufenden Call wechselt erst „Beitreten“ in der Vorschau – kein versehentlicher Wechsel).
  // Ohne Voice auf der Instanz zeigt die Vorschau den Hinweis „nicht eingerichtet“ (statt eines stummen Klicks).
  function onClick() {
    openStage(channel.community_id, channel.id)
    if (voiceEnabled && idle && canConnect) void join(channel)
  }

  return (
    <div className="voice-channel">
      <div className="channel-row">
        <button
          type="button"
          className={`channel${selected ? ' active' : ''}${connected ? ' connected' : ''}${muted ? ' muted-channel' : ''}${voiceEnabled ? '' : ' voice-unavailable'}`}
          onClick={onClick}
          title={title}
          {...menu}
        >
          <span className="channel-icon" aria-hidden="true">
            <Volume2 size={18} />
          </span>
          <span className="truncate">{channel.name}</span>
        </button>
        {onSettings && (
          <button type="button" className="channel-settings icon-button small" title="Kanal bearbeiten" onClick={onSettings}>
            <Settings size={15} />
          </button>
        )}
      </div>
      {participants.length > 0 && (
        <ul className="voice-participants">
          {participants.map((v) => (
            <VoiceParticipant key={v.user_id} state={v} channel={channel} />
          ))}
        </ul>
      )}
    </div>
  )
}

function VoiceParticipant({ state, channel }: { state: VoiceState; channel: Channel }) {
  const me = useStore((s) => s.user)
  const member = useMember(channel.community_id, state.user_id)
  const voice = useVoice()
  const isMe = state.user_id === me?.id
  const inMyRoom = voice.status !== 'idle' && voice.channelId === channel.id
  const speaking = inMyRoom && voice.speaking.includes(state.user_id)

  // Für sich selbst den lokalen Zustand anzeigen (sofortiges Feedback).
  const mute = isMe && inMyRoom ? voice.selfMute || voice.selfDeaf || !voice.canSpeak : state.self_mute
  const deaf = isMe && inMyRoom ? voice.selfDeaf : state.self_deaf
  const streaming = state.streaming || (inMyRoom && voice.streams.includes(state.user_id))
  const camera = inMyRoom ? voice.cameras.includes(state.user_id) || (!isMe && state.video) : state.video
  const watching = voice.watching.includes(state.user_id)

  const name = member ? memberName(member) : isMe ? displayName(me) : '…'
  const avatarUser = member?.user ?? (isMe ? me : null)
  const locallyMuted = useVoice((s) => !isMe && !!s.userAudio[state.user_id]?.muted)
  const menu = useContextMenu(
    (ctx) => buildUserMenu({ userId: state.user_id, communityId: channel.community_id, target: ctx.target, voice: true }),
    name,
  )

  /** Bühne öffnen; Stream ansehen bzw. Kamera fokussieren (dafür ggf. erst beitreten). */
  async function onClick() {
    openStage(channel.community_id, channel.id)
    if (!streaming && !camera) return
    if (!inMyRoom) {
      await voice.join(channel)
      if (useVoice.getState().channelId !== channel.id) return
    }
    const v = useVoice.getState()
    if (streaming) {
      v.watch(state.user_id)
      v.setFocus(tileKey('screen', state.user_id))
    } else v.setFocus(tileKey('user', state.user_id))
  }

  return (
    <li>
      <button
        type="button"
        className={`voice-participant clickable${watching ? ' watching' : ''}`}
        onClick={() => void onClick()}
        title={streaming ? 'Stream ansehen' : camera ? 'Kamera ansehen' : name}
        data-menu-anchor
        {...menu}
      >
        <span className={`voice-avatar${speaking ? ' speaking' : ''}`} aria-hidden="true">
          <Avatar user={avatarUser ?? { id: state.user_id }} name={name} size={22} />
        </span>
        <span className="truncate voice-name">{name}</span>
        {streaming && <span className="live-badge">LIVE</span>}
        {camera && (
          <span className="voice-flag camera" title="Kamera an" aria-label="Kamera an">
            <Video size={14} aria-hidden="true" />
          </span>
        )}
        {locallyMuted && (
          <span className="voice-flag local" title="Für dich stummgeschaltet" aria-label="Für dich stummgeschaltet">
            <VolumeX size={14} aria-hidden="true" />
          </span>
        )}
        {deaf ? (
          <span className="voice-flag deaf" title="Taub geschaltet" aria-label="Taub geschaltet">
            <HeadphoneOff size={14} />
          </span>
        ) : (
          mute && (
            <span className="voice-flag" title="Stumm geschaltet" aria-label="Stumm geschaltet">
              <MicOff size={14} />
            </span>
          )
        )}
      </button>
    </li>
  )
}

