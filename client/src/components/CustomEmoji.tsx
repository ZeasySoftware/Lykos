import { useEffect, useState } from 'react'
import { attachmentUrl, attachmentsNeedToken, fetchAttachment } from '../lib/api'
import { customEmojiPath } from '../lib/customEmoji'
import { findCustomEmoji, useCustomEmojis } from '../lib/emojiData'
import { useStore } from '../store'

/**
 * Object-URLs geladener Emote-Bilder (Desktop: per fetch mit Token). Emote-IDs ändern sich nie, daher darf der
 * Cache für die Laufzeit halten. `null` = nicht ladbar (gelöscht) → Platzhalter `:name:`.
 */
const cache = new Map<string, string | null>()
const pending = new Map<string, Promise<string | null>>()

function load(url: string): Promise<string | null> {
  let p = pending.get(url)
  if (!p) {
    p = fetchAttachment({ url })
      .then((blob) => URL.createObjectURL(blob))
      .catch(() => null)
      .then((src) => {
        cache.set(url, src)
        pending.delete(url)
        return src
      })
    pending.set(url, p)
  }
  return p
}

/** `src` für ein Emote: `undefined` = lädt noch, `null` = nicht verfügbar. */
function useEmojiSrc(url: string): string | null | undefined {
  const direct = attachmentsNeedToken() ? undefined : attachmentUrl({ url })
  const [loaded, setLoaded] = useState<{ url: string; src: string | null } | null>(null)
  useEffect(() => {
    if (direct !== undefined || cache.has(url)) return
    let cancelled = false
    void load(url).then((src) => !cancelled && setLoaded({ url, src }))
    return () => {
      cancelled = true
    }
  }, [url, direct])
  if (direct !== undefined) return direct
  if (cache.has(url)) return cache.get(url)
  return loaded?.url === url ? loaded.src : undefined
}

/**
 * Custom-Emote als Bild. Tooltip `:name:` samt Herkunfts-Community – die kennt der Client nur bei eigenen
 * Communities (siehe Datenschutz-Abwägung in docs/api.md). Gelöschte Emotes erscheinen als `:name:`.
 */
export function CustomEmojiImage({
  id,
  name,
  animated,
  size = 22,
  tooltip = true,
}: {
  id: string
  name: string
  animated: boolean
  size?: number
  tooltip?: boolean
}) {
  const src = useEmojiSrc(customEmojiPath(id, animated))
  const [failed, setFailed] = useState(false)
  const byCommunity = useCustomEmojis()
  const known = findCustomEmoji(byCommunity, id)
  const community = useStore((s) => (known ? s.communities.find((c) => c.id === known.community_id) : undefined))

  if (src === null || failed) return <span className="custom-emoji-missing">{`:${name}:`}</span>
  const label = `:${known?.name ?? name}:`
  const title = !tooltip
    ? undefined
    : community
      ? `${label}\naus ${community.name}`
      : `${label}\nEmoji aus einer anderen Community`
  if (src === undefined) {
    return <span className="custom-emoji pending" style={{ width: size, height: size }} title={title} aria-label={label} />
  }
  return (
    <img
      className="custom-emoji"
      src={src}
      alt={label}
      title={title}
      width={size}
      height={size}
      draggable={false}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  )
}
