import { useCallback, useState } from 'react'
import type { Message } from '../lib/api'
import { customReaction, parseCustomReaction } from '../lib/customEmoji'
import { errorText } from '../lib/errors'
import { useStore } from '../store'
import { SmilePlus } from 'lucide-react'
import { CustomEmojiImage } from './CustomEmoji'
import { EmojiPicker as FullEmojiPicker } from './EmojiPicker'

/**
 * Emoji-Picker für Reaktionen: liefert das Emoji als API-Schlüssel (Unicode-Emoji bzw. `name:id` bei
 * Custom-Emotes). Positioniert sich am Elternelement.
 */
export function EmojiPicker({ onPick, onClose }: { onPick: (emoji: string) => void; onClose: () => void }) {
  return (
    <FullEmojiPicker
      custom
      label="Reaktion wählen"
      onClose={onClose}
      onPick={(p) => onPick(p.kind === 'custom' ? customReaction(p.emoji) : p.emoji)}
    />
  )
}

/** Anzeige eines Reaktions-Emojis (Unicode oder Custom-Emote). */
export function ReactionEmoji({ emoji, animated = false }: { emoji: string; animated?: boolean }) {
  const custom = parseCustomReaction(emoji)
  if (custom) return <CustomEmojiImage id={custom.id} name={custom.name} animated={animated} size={18} />
  return <span className="reaction-emoji">{emoji}</span>
}

/** Reaktionsleiste unter einer Nachricht; Klick schaltet die eigene Reaktion um. */
export function ReactionBar({ message, canReact }: { message: Message; canReact: boolean }) {
  const toggleReaction = useStore((s) => s.toggleReaction)
  const [picker, setPicker] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const closePicker = useCallback(() => setPicker(false), [])
  const reactions = message.reactions ?? []
  if (reactions.length === 0) return null

  function toggle(emoji: string) {
    toggleReaction(message, emoji).then(
      () => setError(null),
      (e) => setError(errorText(e)),
    )
  }

  return (
    <div className="reactions">
      {reactions.map((r) => {
        const custom = parseCustomReaction(r.emoji)
        return (
          <button
            key={custom?.id ?? r.emoji}
            type="button"
            className={`reaction${r.me ? ' me' : ''}`}
            aria-pressed={r.me}
            aria-label={`${custom ? `:${custom.name}:` : r.emoji} ${r.count}`}
            // Eigene Reaktion entfernen geht immer, neue nur mit ADD_REACTIONS.
            disabled={!canReact && !r.me}
            title={`${custom ? `:${custom.name}: – ` : ''}${r.me ? 'Reaktion entfernen' : 'Ebenfalls reagieren'}`}
            onClick={() => toggle(r.emoji)}
          >
            <ReactionEmoji emoji={r.emoji} animated={r.animated} />
            <span className="reaction-count">{r.count}</span>
          </button>
        )
      })}
      {canReact && (
        <span className="reaction-add-wrap">
          <button
            type="button"
            className="reaction reaction-add"
            title="Reaktion hinzufügen"
            onClick={() => setPicker((p) => !p)}
          >
            <SmilePlus size={16} />
          </button>
          {picker && <EmojiPicker onPick={toggle} onClose={closePicker} />}
        </span>
      )}
      {error && <span className="error small">{error}</span>}
    </div>
  )
}
