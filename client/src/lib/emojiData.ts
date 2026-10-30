import { useEffect, useMemo } from 'react'
import { create } from 'zustand'
import { endpoints } from './api'
import type { CustomEmoji } from './api'
import { prepareEmojis, pushRecent } from './emojiSearch'
import type { RawEmojiRow, UnicodeEmoji } from './emojiSearch'
import { useStore } from '../store'

// ---------------------------------------------------------------------------
// Unicode-Emojis: statisches JSON im Bundle, erst bei Bedarf geladen (eigener Chunk)
// ---------------------------------------------------------------------------

let unicodePromise: Promise<UnicodeEmoji[]> | null = null
const useUnicodeStore = create<{ data: UnicodeEmoji[] | null }>()(() => ({ data: null }))

/** Lädt die Emoji-Daten (einmal pro Sitzung). */
export function loadEmojiData(): Promise<UnicodeEmoji[]> {
  unicodePromise ??= import('../data/emoji-data.json')
    .then((m) => {
      const data = prepareEmojis((m.default as unknown as { emojis: RawEmojiRow[] }).emojis)
      useUnicodeStore.setState({ data })
      return data
    })
    .catch((e) => {
      unicodePromise = null
      throw e
    })
  return unicodePromise
}

/**
 * Emoji-Daten, sobald geladen (`null` vorher). `enabled = false` stößt das Laden nicht an, liefert die Daten
 * aber, sobald sie geladen sind.
 */
export function useEmojiData(enabled = true): UnicodeEmoji[] | null {
  const data = useUnicodeStore((s) => s.data)
  useEffect(() => {
    if (enabled && !data) loadEmojiData().catch(() => {})
  }, [enabled, data])
  return data
}

// ---------------------------------------------------------------------------
// Vorlieben: Hautton und „Zuletzt verwendet“ (lokal, localStorage kann fehlen)
// ---------------------------------------------------------------------------

const SKIN_KEY = 'lykos_emoji_skin'
const RECENT_KEY = 'lykos_emoji_recent'

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Speicher nicht verfügbar – gilt dann nur für diese Sitzung.
  }
}

function readRecent(): string[] {
  try {
    const list = JSON.parse(read(RECENT_KEY) ?? '[]')
    return Array.isArray(list) ? list.filter((k): k is string => typeof k === 'string').slice(0, 27) : []
  } catch {
    return []
  }
}

interface EmojiPrefs {
  /** 0 = Standard (gelb), 1–5 = hell → dunkel. */
  skin: number
  /** Zuletzt verwendete Emojis: Unicode-Basis-Emoji oder `custom:{id}`. */
  recent: string[]
  setSkin: (skin: number) => void
  addRecent: (key: string) => void
}

export const useEmojiPrefs = create<EmojiPrefs>()((set) => ({
  skin: Math.min(5, Math.max(0, Number(read(SKIN_KEY)) || 0)),
  recent: readRecent(),
  setSkin: (skin) => {
    write(SKIN_KEY, String(skin))
    set({ skin })
  },
  addRecent: (key) =>
    set((s) => {
      const recent = pushRecent(s.recent, key)
      write(RECENT_KEY, JSON.stringify(recent))
      return { recent }
    }),
}))

// ---------------------------------------------------------------------------
// Custom-Emotes der eigenen Communities
// ---------------------------------------------------------------------------

interface CustomEmojiState {
  /** community_id → Emotes (alphabetisch). */
  byCommunity: Record<string, CustomEmoji[]>
}

export const useCustomEmojiStore = create<CustomEmojiState>()(() => ({ byCommunity: {} }))

/** Gateway `EMOJIS_UPDATE`: Liste einer Community ersetzen. */
export function applyEmojisUpdate(d: { community_id: string; emojis: CustomEmoji[] }) {
  useCustomEmojiStore.setState((s) => ({ byCommunity: { ...s.byCommunity, [d.community_id]: d.emojis } }))
}

let syncing = false
let reloadTimer: number | undefined

function reload() {
  window.clearTimeout(reloadTimer)
  reloadTimer = window.setTimeout(() => {
    if (!useStore.getState().user) {
      useCustomEmojiStore.setState({ byCommunity: {} })
      return
    }
    endpoints.myEmojis().then(
      (list) => {
        const byCommunity: Record<string, CustomEmoji[]> = {}
        for (const e of list) (byCommunity[e.community_id] ??= []).push(e)
        useCustomEmojiStore.setState({ byCommunity })
      },
      () => {},
    )
  }, 250)
}

/**
 * Hält die Emote-Liste aktuell: lädt sie einmal und erneut, sobald sich die Community-Liste ändert (Beitritt,
 * Verlassen, Neuladen nach Reconnect) oder der Nutzer wechselt. Änderungen innerhalb einer Community kommen per
 * `EMOJIS_UPDATE`. Lazy gestartet, damit es keinen Import-Zyklus beim Laden von `store.ts` gibt.
 */
export function ensureCustomEmojiSync() {
  if (syncing) return
  syncing = true
  reload()
  useStore.subscribe((s, prev) => {
    if (s.communities !== prev.communities || s.user?.id !== prev.user?.id) reload()
  })
}

/** Emotes der eigenen Communities, gruppiert nach Community. */
export function useCustomEmojis(): Record<string, CustomEmoji[]> {
  useEffect(ensureCustomEmojiSync, [])
  return useCustomEmojiStore((s) => s.byCommunity)
}

/** Alle eigenen Emotes als flache Liste. */
export function useCustomEmojiList(): CustomEmoji[] {
  const byCommunity = useCustomEmojis()
  return useMemo(() => Object.values(byCommunity).flat(), [byCommunity])
}

/** Eigenes Emote nach ID (für Tooltips mit Herkunfts-Community); `undefined` bei fremden Emotes. */
export function findCustomEmoji(byCommunity: Record<string, CustomEmoji[]>, id: string): CustomEmoji | undefined {
  for (const list of Object.values(byCommunity)) {
    const hit = list.find((e) => e.id === id)
    if (hit) return hit
  }
  return undefined
}
