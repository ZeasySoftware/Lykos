import { useRef } from 'react'
import type { CSSProperties, KeyboardEvent } from 'react'
import { SmilePlus } from 'lucide-react'
import type { CustomEmoji, Message } from '../lib/api'
import { MAX_VOLUME } from '../lib/audioLogic'
import { customReaction } from '../lib/customEmoji'
import { useCustomEmojis, useEmojiPrefs } from '../lib/emojiData'
import { useMenu } from '../lib/menuStore'
import { toastError } from '../lib/toast'
import { DEFAULT_USER_AUDIO, useVoice } from '../lib/voice'
import { useStore } from '../store'
import { CustomEmojiImage } from './CustomEmoji'

/** Standard-Schnellreaktionen (Emojis sind hier Inhalt, keine UI-Icons). */
const QUICK_REACTIONS = ['👍', '❤️', '😂', '😮', '🎉']
/** So viele Schnellreaktionen insgesamt; vorne die zuletzt verwendeten Custom-Emotes. */
const QUICK_COUNT = 5
const MAX_RECENT_CUSTOM = 2

interface QuickItem {
  /** API-Schlüssel der Reaktion (Unicode oder `name:id`). */
  key: string
  /** Schlüssel für „Zuletzt verwendet“. */
  recent: string
  label: string
  custom?: CustomEmoji
}

/**
 * Zeile mit Schnellreaktionen; vorne zuletzt verwendete Custom-Emotes der Community (wie im Picker aus
 * Paket I), dann Standard-Emojis. Pfeil links/rechts wechselt zwischen den Einträgen; „Weitere…“ öffnet
 * den vollen Emoji-Picker.
 */
export function QuickReactions({ message, onMore }: { message: Message; onMore: () => void }) {
  const close = useMenu((s) => s.close)
  const toggle = useStore((s) => s.toggleReaction)
  const recent = useEmojiPrefs((s) => s.recent)
  const addRecent = useEmojiPrefs((s) => s.addRecent)
  const byCommunity = useCustomEmojis()
  const mine = new Set((message.reactions ?? []).filter((r) => r.me).map((r) => r.emoji))
  const row = useRef<HTMLDivElement>(null)

  // Custom-Emotes nur aus der Community der Nachricht (in DMs keine).
  const available = new Map((message.community_id ? (byCommunity[message.community_id] ?? []) : []).map((e) => [e.id, e]))
  const customs: QuickItem[] = recent
    .filter((k) => k.startsWith('custom:'))
    .map((k) => available.get(k.slice(7)))
    .filter((e): e is CustomEmoji => !!e)
    .slice(0, MAX_RECENT_CUSTOM)
    .map((e) => ({ key: customReaction(e), recent: `custom:${e.id}`, label: `:${e.name}:`, custom: e }))
  const items: QuickItem[] = [
    ...customs,
    ...QUICK_REACTIONS.map((e) => ({ key: e, recent: e, label: e })),
  ].slice(0, QUICK_COUNT)

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
    e.preventDefault()
    const buttons = [...(row.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])]
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = (i + (e.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length
    buttons[next]?.focus()
  }

  return (
    <div className="ctx-reactions" ref={row} onKeyDown={onKeyDown} role="group" aria-label="Schnellreaktionen">
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          tabIndex={-1}
          className={`ctx-reaction${mine.has(item.key) ? ' me' : ''}`}
          title={item.custom ? item.label : undefined}
          aria-label={mine.has(item.key) ? `Reaktion ${item.label} entfernen` : `Mit ${item.label} reagieren`}
          onClick={() => {
            close()
            addRecent(item.recent)
            toggle(message, item.key).catch(toastError)
          }}
        >
          {item.custom ? (
            <CustomEmojiImage id={item.custom.id} name={item.custom.name} animated={item.custom.animated} size={22} tooltip={false} />
          ) : (
            item.label
          )}
        </button>
      ))}
      <button
        type="button"
        tabIndex={-1}
        className="ctx-reaction more"
        title="Weitere Reaktionen …"
        aria-label="Weitere Reaktionen"
        onClick={() => {
          close()
          onMore()
        }}
      >
        <SmilePlus size={18} />
      </button>
    </div>
  )
}

/** Regler für die lokale Lautstärke eines Voice-Teilnehmers (nur auf diesem Gerät, bis 200 %). */
export function VolumeSlider({ userId }: { userId: string }) {
  const audio = useVoice((s) => s.userAudio[userId]) ?? DEFAULT_USER_AUDIO
  const setVolume = useVoice((s) => s.setUserVolume)
  const percent = Math.round(audio.volume * 100)
  const max = MAX_VOLUME * 100
  return (
    <label className="ctx-slider">
      <span className="ctx-slider-head">
        <span>Lautstärke</span>
        <span className="muted">{audio.muted ? 'stumm' : `${percent} %`}</span>
      </span>
      <input
        type="range"
        min={0}
        max={max}
        step={1}
        value={percent}
        tabIndex={-1}
        aria-label="Lautstärke für mich"
        style={{ '--fill': `${(percent / max) * 100}%` } as CSSProperties}
        onChange={(e) => setVolume(userId, Number(e.target.value) / 100)}
      />
    </label>
  )
}
