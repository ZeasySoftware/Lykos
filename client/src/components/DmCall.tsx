/**
 * Anrufe in DMs: Knöpfe im DM-Kopf, Anrufbereich über dem Verlauf (Bühne bzw. „Laufender Anruf“),
 * Karten für eingehende Anrufe und die Systemnachricht „Anruf verpasst“.
 */
import { Suspense, useMemo } from 'react'
import { Phone, PhoneIncoming, PhoneMissed, PhoneOff, Video } from 'lucide-react'
import type { DmChannel, Message } from '../lib/api'
import { acceptCall, declineCall, ringAgain, startCall } from '../lib/callActions'
import { incomingCalls, isCallActive, isCallingOut, joinNames } from '../lib/callLogic'
import { useCall, useCalls } from '../lib/callStore'
import { useFriends } from '../lib/friendsStore'
import { runAction } from '../lib/toast'
import { useVoice } from '../lib/voice'
import { lazyComponent } from '../lib/lazy'
import { displayName, dmTitle, useStore } from '../store'
import { Avatar } from './Presence'

// Die Call-Bühne liegt in einem eigenen Chunk (siehe lib/lazy.ts) – auch hier nur bei Bedarf laden.
const CallRoom = lazyComponent(() => import('./CallStage').then((m) => m.CallRoom))

const timeFmt = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' })

/** Ist man selbst im Anruf (bzw. verbindet gerade) dieser DM? */
function useInCall(channelId: string): boolean {
  return useVoice((s) => s.status !== 'idle' && s.channelId === channelId)
}

/** 1:1-DM mit einer selbst blockierten Person: kein Anruf (der Server sperrt auch die Gegenrichtung). */
function usePartnerBlocked(dm: DmChannel | undefined, meId: string | undefined): boolean {
  const partner = dm?.kind === 'dm' ? dm.recipients.find((u) => u.id !== meId) : undefined
  return useFriends((s) => !!partner && s.relationships[partner.id]?.type === 'blocked')
}

function nameOf(dm: DmChannel | undefined, userId: string | null | undefined): string {
  const u = dm?.recipients.find((r) => r.id === userId)
  return u ? displayName(u) : 'Jemand'
}

// ---------------------------------------------------------------------------
// Kopf der DM: Anrufen / Videoanruf
// ---------------------------------------------------------------------------

/** Knöpfe „Anrufen“ und „Videoanruf“ im Kopf einer DM (ausgeblendet, solange man selbst drin ist). */
export function DmCallButtons({ channelId }: { channelId: string }) {
  const dm = useStore((s) => s.dms[channelId])
  const me = useStore((s) => s.user?.id)
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const inCall = useInCall(channelId)
  const active = isCallActive(useCall(channelId))
  const blocked = usePartnerBlocked(dm, me)
  if (!dm || !voiceEnabled || inCall) return null
  const title = blocked
    ? 'Du hast diese Person blockiert'
    : active
      ? 'Laufendem Anruf beitreten'
      : dm.kind === 'dm'
        ? 'Anrufen'
        : 'Gruppenanruf starten'
  return (
    <>
      <button
        type="button"
        className="icon-button"
        title={title}
        aria-label={title}
        disabled={blocked}
        onClick={() => runAction(() => startCall(channelId))}
      >
        <Phone size={18} />
      </button>
      <button
        type="button"
        className="icon-button"
        title={blocked ? title : active ? 'Mit Kamera beitreten' : 'Videoanruf starten'}
        aria-label={active ? 'Mit Kamera beitreten' : 'Videoanruf starten'}
        disabled={blocked}
        onClick={() => runAction(() => startCall(channelId, { video: true }))}
      >
        <Video size={18} />
      </button>
    </>
  )
}

// ---------------------------------------------------------------------------
// Anrufbereich über dem Verlauf
// ---------------------------------------------------------------------------

/**
 * Über dem Verlauf einer DM: die Call-Bühne, solange man im Anruf ist; sonst – wenn ein Anruf läuft oder es
 * bei einem klingelt – ein Hinweis mit „Beitreten“. Ohne Anruf nichts.
 */
export function DmCallSection({ channelId }: { channelId: string }) {
  const dm = useStore((s) => s.dms[channelId])
  const me = useStore((s) => s.user?.id)
  const call = useCall(channelId)
  const inCall = useInCall(channelId)
  const room = useMemo(
    () => ({ id: channelId, communityId: null, name: dm ? dmTitle(dm, me) : 'Anruf' }),
    [channelId, dm, me],
  )
  if (!dm) return null
  if (inCall) {
    return (
      <div className="dm-call">
        <Suspense fallback={null}>
          <CallRoom room={room} embedded banner={<OutgoingBanner dm={dm} />} />
        </Suspense>
      </div>
    )
  }
  if (!isCallActive(call) && !(me && call?.ringing.includes(me))) return null
  return <OngoingCallBar dm={dm} />
}

/** Im eigenen Anruf: „Klingelt bei …“ bzw. „Niemand hat angenommen – erneut klingeln“. */
function OutgoingBanner({ dm }: { dm: DmChannel }) {
  const me = useStore((s) => s.user?.id)
  const call = useCall(dm.id)
  const connected = useVoice((s) => s.status === 'connected')
  if (isCallingOut(call, me, true)) {
    const names = joinNames((call?.ringing ?? []).map((id) => nameOf(dm, id)))
    return (
      <div className="call-banner call-ringing" role="status">
        <span className="call-ringing-text">
          <PhoneIncoming size={16} aria-hidden="true" />
          Klingelt bei {names} …
        </span>
      </div>
    )
  }
  // Allein im Anruf, es klingelt nirgends mehr: erneut anrufen anbieten.
  const alone = !(call?.voiceStates ?? []).some((v) => v.user_id !== me)
  if (!connected || !alone || dm.recipients.length < 2) return null
  return (
    <div className="call-banner call-ringing" role="status">
      <span className="call-ringing-text">Niemand hat angenommen.</span>
      <button type="button" className="button small-button" onClick={() => ringAgain(dm.id)}>
        <Phone size={14} aria-hidden="true" /> Erneut klingeln
      </button>
    </div>
  )
}

/** Hinweis über dem Verlauf: Anruf läuft (oder es klingelt bei einem) – beitreten. */
function OngoingCallBar({ dm }: { dm: DmChannel }) {
  const me = useStore((s) => s.user?.id)
  const call = useCall(dm.id)
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const states = call?.voiceStates ?? []
  const ringingMe = !!me && !!call?.ringing.includes(me)
  const people = states.map((v) => dm.recipients.find((u) => u.id === v.user_id) ?? { id: v.user_id })
  const label = ringingMe
    ? `${nameOf(dm, call?.callerId)} ruft an …`
    : states.length === 1
      ? `${nameOf(dm, states[0].user_id)} ist im Anruf`
      : `${joinNames(states.map((v) => nameOf(dm, v.user_id)))} sind im Anruf`
  return (
    <div className={`dm-call-bar${ringingMe ? ' ringing' : ''}`} role="status">
      <span className="dm-call-bar-avatars" aria-hidden="true">
        {people.slice(0, 4).map((u) => (
          <Avatar key={u.id} user={u} name={'username' in u ? displayName(u) : '…'} size={28} />
        ))}
      </span>
      <span className="dm-call-bar-text">
        <strong>{ringingMe ? 'Eingehender Anruf' : 'Laufender Anruf'}</strong>
        <span className="muted small truncate">{label}</span>
      </span>
      <span className="dm-call-bar-actions">
        {ringingMe && (
          <button type="button" className="call-action decline" title="Ablehnen" aria-label="Ablehnen" onClick={() => declineCall(dm.id)}>
            <PhoneOff size={18} />
          </button>
        )}
        <button
          type="button"
          className="call-action accept"
          title="Mit Kamera beitreten"
          aria-label="Mit Kamera beitreten"
          disabled={!voiceEnabled}
          onClick={() => acceptCall(dm.id, true)}
        >
          <Video size={18} />
        </button>
        <button
          type="button"
          className="button primary call-join-button"
          disabled={!voiceEnabled}
          onClick={() => acceptCall(dm.id)}
        >
          <Phone size={16} aria-hidden="true" /> {ringingMe ? 'Annehmen' : 'Beitreten'}
        </button>
      </span>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Eingehende Anrufe (überall sichtbar, nicht modal)
// ---------------------------------------------------------------------------

/**
 * Karten für eingehende Anrufe, oben rechts (mobil oben über die volle Breite). Ist die DM des Anrufs gerade
 * offen, übernimmt die Leiste über dem Verlauf (gleiche Knöpfe) – dort keine zusätzliche Karte.
 */
export function IncomingCalls() {
  const calls = useCalls((s) => s.calls)
  const hidden = useCalls((s) => s.hidden)
  const me = useStore((s) => s.user?.id)
  const openDm = useStore((s) => (s.dmView ? s.selectedDmId : null))
  const current = useVoice((s) => (s.status === 'idle' ? null : s.channelId))
  const incoming = useMemo(
    () => incomingCalls(calls, me, { currentChannelId: current, hidden }).filter((c) => c.channelId !== openDm),
    [calls, me, current, hidden, openDm],
  )
  if (incoming.length === 0) return null
  return (
    <div className="incoming-calls" role="region" aria-label="Eingehende Anrufe">
      {incoming.map((c) => (
        <IncomingCallCard key={c.channelId} channelId={c.channelId} callerId={c.callerId} />
      ))}
    </div>
  )
}

function IncomingCallCard({ channelId, callerId }: { channelId: string; callerId: string | null }) {
  const dm = useStore((s) => s.dms[channelId])
  const me = useStore((s) => s.user?.id)
  const selectDm = useStore((s) => s.selectDm)
  const inOtherCall = useVoice((s) => s.status !== 'idle')
  const voiceEnabled = useStore((s) => s.instance?.voice ?? false)
  const caller = dm?.recipients.find((u) => u.id === callerId)
  const name = caller ? displayName(caller) : 'Jemand'
  const where = dm?.kind === 'group_dm' ? dmTitle(dm, me) : null

  return (
    <div className="incoming-call" role="alertdialog" aria-label={`Eingehender Anruf von ${name}`}>
      <button type="button" className="incoming-call-who" onClick={() => selectDm(channelId)} title="Unterhaltung öffnen">
        <span className="incoming-call-avatar" aria-hidden="true">
          <Avatar user={caller ?? { id: callerId ?? channelId }} name={name} size={48} />
        </span>
        <span className="incoming-call-text">
          <strong className="truncate">{name}</strong>
          <span className="muted small truncate">{where ? `ruft in „${where}“ an …` : 'ruft dich an …'}</span>
          {inOtherCall && <span className="muted small incoming-call-note">Annehmen beendet deinen aktuellen Anruf.</span>}
        </span>
      </button>
      <span className="incoming-call-actions">
        <button
          type="button"
          className="call-action decline"
          title="Ablehnen"
          aria-label="Ablehnen"
          onClick={() => declineCall(channelId)}
        >
          <PhoneOff size={20} />
        </button>
        <button
          type="button"
          className="call-action accept"
          title="Mit Kamera annehmen"
          aria-label="Mit Kamera annehmen"
          disabled={!voiceEnabled}
          onClick={() => acceptCall(channelId, true)}
        >
          <Video size={20} />
        </button>
        <button
          type="button"
          className="call-action accept"
          title="Annehmen"
          aria-label="Annehmen"
          disabled={!voiceEnabled}
          onClick={() => acceptCall(channelId)}
        >
          <Phone size={20} />
        </button>
      </span>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Systemnachricht „Anruf verpasst“
// ---------------------------------------------------------------------------

/** `call_missed` in einer 1:1-DM: Für den Angerufenen „Verpasster Anruf von X“, für den Anrufer „nicht angenommen“. */
export function CallMissedMessage({ message }: { message: Message }) {
  const me = useStore((s) => s.user?.id)
  const dm = useStore((s) => s.dms[message.channel_id])
  const own = !!me && message.author?.id === me
  const partner = dm?.recipients.find((u) => u.id !== me)
  const text = own
    ? `${partner ? displayName(partner) : 'Dein Gegenüber'} hat deinen Anruf nicht angenommen.`
    : `Verpasster Anruf von ${message.author ? displayName(message.author) : 'Gelöschter Nutzer'}.`
  return (
    <div className="message system-message call-missed" data-message-id={message.id}>
      <PhoneMissed size={16} className="system-message-icon" aria-hidden="true" />
      <span className="system-message-text">{text}</span>
      <time className="message-time" dateTime={message.created_at}>
        {timeFmt.format(new Date(message.created_at))}
      </time>
    </div>
  )
}
