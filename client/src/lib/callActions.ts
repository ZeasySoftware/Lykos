/**
 * Aktionen rund um Anrufe in DMs: anrufen, annehmen, ablehnen, erneut klingeln lassen.
 *
 * Anrufen und Annehmen sind beides ein Beitritt zum Raum der DM (`voice.join` mit `community_id: null`);
 * der Server entscheidet, ob damit ein Anruf startet (Raum leer, es klingelt nirgends) oder man annimmt.
 * Es gibt höchstens einen Raum pro Nutzer – ein laufender Sprachkanal/Anruf wird dabei verlassen.
 */
import { endpoints } from './api'
import { useCalls } from './callStore'
import { toastError } from './toast'
import { useVoice } from './voice'
import { useStore } from '../store'

/** Anruf in einer DM starten bzw. einem laufenden beitreten. */
export async function startCall(channelId: string, opts: { video?: boolean } = {}): Promise<void> {
  await useVoice.getState().join({ id: channelId, community_id: null }, opts)
}

/** Eingehenden Anruf annehmen: DM öffnen und beitreten (optional gleich mit Kamera). */
export function acceptCall(channelId: string, video = false) {
  useCalls.getState().hide(channelId)
  useStore.getState().selectDm(channelId)
  void startCall(channelId, { video })
}

/** Eingehenden Anruf ablehnen (Karte verschwindet sofort, der Server beendet das Klingeln). */
export function declineCall(channelId: string) {
  useCalls.getState().hide(channelId)
  endpoints.callDecline(channelId).catch(toastError)
}

/** Bei allen, die noch nicht im Anruf sind, erneut klingeln lassen. */
export function ringAgain(channelId: string) {
  endpoints.callRing(channelId).catch(toastError)
}

/** Nutzer anrufen: 1:1-DM öffnen bzw. anlegen und den Anruf starten. */
export async function callUser(userId: string, opts: { video?: boolean } = {}): Promise<void> {
  await useStore.getState().openDm([userId])
  const channelId = useStore.getState().selectedDmId
  if (channelId) await startCall(channelId, opts)
}
