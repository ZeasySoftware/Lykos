/**
 * Gemeinsame Helfer der Call-Bühne (Bühne, Kacheln, Mini-Ansicht, Kanalliste):
 * Navigation zur Bühne und die Teilnehmerliste eines Sprachkanals bzw. DM-Anrufs (`communityId = null`).
 */
import { useMemo } from 'react'
import type { VoiceState } from './api'
import { useCallStates } from './callStore'
import { useVoice } from './voice'
import { displayName, memberName, useStore } from '../store'

/** Was eine Kachel über ihren Teilnehmer wissen muss. */
export interface CallMember {
  userId: string
  name: string
  user: { id: string; avatar_url?: string | null } | null
  mute: boolean
  deaf: boolean
  camera: boolean
  streaming: boolean
  isMe: boolean
}

const EMPTY: VoiceState[] = []

/** Zur Call-Bühne wechseln: Sprachkanal (Community + Kanal auswählen) bzw. DM des Anrufs. */
export function openStage(communityId: string | null, channelId: string) {
  const app = useStore.getState()
  if (communityId === null) {
    app.selectDm(channelId)
    return
  }
  if (app.dmView || app.selectedCommunityId !== communityId) app.selectCommunity(communityId)
  useStore.getState().selectChannel(channelId)
}

/** Ist die Bühne des eigenen Calls gerade im Hauptbereich zu sehen (bei DM-Anrufen: die DM ist offen)? */
export function useStageVisible(): boolean {
  const channelId = useVoice((s) => (s.status === 'idle' ? null : s.channelId))
  const dm = useVoice((s) => s.communityId === null)
  return useStore((s) =>
    !channelId ? false : dm ? s.dmView && s.selectedDmId === channelId : !s.dmView && s.selectedChannelId === channelId,
  )
}

export function toggleFullscreen(el: HTMLElement | null) {
  if (!el) return
  if (document.fullscreenElement) void document.exitFullscreen().catch(() => {})
  else void el.requestFullscreen().catch(() => {})
}

/**
 * Teilnehmer eines Sprachkanals für Kacheln: Voice-States vom Server (Reihenfolge nach Beitritt),
 * im eigenen Raum ergänzt um LiveKit-Teilnehmer, die der Server noch nicht gemeldet hat, und mit dem
 * lokalen Stand für Kamera/Stream (sofort, ohne auf den Webhook zu warten).
 */
export function useCallMembers(communityId: string | null, channelId: string | null): CallMember[] {
  const communityStates = useStore((s) => (communityId ? (s.voiceStates[communityId] ?? EMPTY) : EMPTY))
  const dmStates = useCallStates(communityId ? null : channelId)
  const states = communityId ? communityStates : dmStates
  const members = useStore((s) => (communityId ? s.members[communityId] : undefined))
  // In DMs sind die Teilnehmer der DM die „Mitglieder“.
  const recipients = useStore((s) => (communityId || !channelId ? undefined : s.dms[channelId]?.recipients))
  const me = useStore((s) => s.user)
  const inRoom = useVoice((s) => s.status !== 'idle' && s.channelId === channelId)
  const roomIds = useVoice((s) => s.participants)
  const cameras = useVoice((s) => s.cameras)
  const streams = useVoice((s) => s.streams)
  const selfMute = useVoice((s) => s.selfMute || s.selfDeaf || !s.canSpeak)
  const selfDeaf = useVoice((s) => s.selfDeaf)

  return useMemo(() => {
    const list = states
      .filter((v) => v.channel_id === channelId)
      .slice()
      .sort((a, b) => (a.joined_at ?? '').localeCompare(b.joined_at ?? ''))
    const byId = new Map(list.map((v) => [v.user_id, v]))
    const ids = list.map((v) => v.user_id)
    if (inRoom) for (const id of roomIds) if (!byId.has(id)) ids.push(id)
    return ids.map((userId): CallMember => {
      const state = byId.get(userId)
      const member = members?.find((m) => m.user.id === userId)
      const recipient = recipients?.find((u) => u.id === userId)
      const isMe = userId === me?.id
      return {
        userId,
        name: member ? memberName(member) : recipient ? displayName(recipient) : isMe ? displayName(me) : '…',
        user: member?.user ?? recipient ?? (isMe ? me : null),
        isMe,
        mute: isMe && inRoom ? selfMute : (state?.self_mute ?? false),
        deaf: isMe && inRoom ? selfDeaf : (state?.self_deaf ?? false),
        camera: inRoom ? cameras.includes(userId) : !!state?.video,
        streaming: inRoom ? streams.includes(userId) : !!state?.streaming,
      }
    })
  }, [states, channelId, members, recipients, me, inRoom, roomIds, cameras, streams, selfMute, selfDeaf])
}
