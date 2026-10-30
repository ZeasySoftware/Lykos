/**
 * Automatischer Wechsel in den AFK-Kanal: misst die eigene Inaktivität im Sprachkanal (kein Sprechen laut
 * Sprachaktivierung/Push-to-Talk, keine Eingabe in der App) und tritt nach der Zeit der Community über den
 * normalen Beitrittsweg dem AFK-Kanal bei. Regeln in `afk.ts`.
 */
import { useEffect } from 'react'
import { afkTimeoutLabel, nextLastActivity, shouldMoveToAfk } from './afk'
import { Permission, has } from './permissions'
import { toast } from './toast'
import { useVoice } from './voice'
import { useStore } from '../store'

/** Prüfintervall – die kürzeste AFK-Zeit ist eine Minute. */
const CHECK_MS = 5_000

export function useAfkMove() {
  useEffect(() => {
    let lastActivity = Date.now()
    const activity = () => {
      lastActivity = Date.now()
    }
    const events = ['pointermove', 'pointerdown', 'keydown', 'wheel', 'touchstart'] as const
    for (const e of events) window.addEventListener(e, activity, { passive: true })

    // Kanalwechsel und Push-to-Talk sofort als Aktivität werten.
    const unsubscribe = useVoice.subscribe((s, prev) => {
      if (s.channelId !== prev.channelId || (s.pttPressed && !prev.pttPressed)) activity()
    })

    const timer = window.setInterval(() => {
      const now = Date.now()
      const v = useVoice.getState()
      const app = useStore.getState()
      lastActivity = nextLastActivity(lastActivity, now, {
        speaking: v.speaking,
        selfId: app.user?.id,
        pttPressed: v.pttPressed,
      })
      const detail = v.communityId ? app.details[v.communityId] : undefined
      const afkChannelId = detail?.community.afk_channel_id ?? null
      const afkPerms = afkChannelId ? detail?.channel_permissions[afkChannelId] : undefined
      const move = shouldMoveToAfk({
        now,
        lastActivity,
        status: v.status,
        communityId: v.communityId,
        channelId: v.channelId,
        afkChannelId,
        afkTimeoutSeconds: detail?.community.afk_timeout_seconds,
        screenSharing: v.screenSharing,
        cameraOn: v.cameraOn,
        canJoinAfk: afkPerms !== undefined && has(afkPerms, Permission.CONNECT),
      })
      if (!move || !afkChannelId || !detail) return
      lastActivity = now
      const channel = detail.channels.find((c) => c.id === afkChannelId)
      void v.join({ id: afkChannelId, community_id: detail.community.id })
      toast(
        `Nach ${afkTimeoutLabel(detail.community.afk_timeout_seconds)} ohne Aktivität in den AFK-Kanal${channel ? ` „${channel.name}“` : ''} verschoben.`,
      )
    }, CHECK_MS)

    return () => {
      window.clearInterval(timer)
      unsubscribe()
      for (const e of events) window.removeEventListener(e, activity)
    }
  }, [])
}
