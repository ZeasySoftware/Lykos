import { useCallback, useRef, useState } from 'react'
import type { InputHTMLAttributes, RefObject, TextareaHTMLAttributes } from 'react'
import { Smile } from 'lucide-react'
import type { CustomEmoji } from '../lib/api'
import { insertIntoField, pickedText } from '../lib/emojiInsert'
import type { Field } from '../lib/emojiInsert'
import { EmojiPicker } from './EmojiPicker'
import type { PickedEmoji } from './EmojiPicker'

const FIELD_SELECTOR =
  'textarea, input:not([type=checkbox]):not([type=radio]):not([type=file]):not([type=range]):not([type=number])'

/**
 * Smiley-Button, der einen Emoji-Picker öffnet und das Gewählte an der Cursorposition einfügt – in `target` oder,
 * ohne `target`, in das Eingabefeld im selben Elternelement. So lässt er sich mit einer Zeile neben ein bestehendes
 * `<input>`/`<textarea>` hängen. `custom` bietet Custom-Emotes an (nur für Nachrichten sinnvoll); `onPick` ersetzt
 * das Einfügen.
 */
export function EmojiButton({
  target,
  custom = false,
  customText,
  onPick,
  disabled,
  className = '',
  size = 18,
  title = 'Emoji einfügen',
}: {
  target?: RefObject<Field | null>
  custom?: boolean
  /** Einfügetext für Custom-Emotes (Standard: `<:name:id>`). */
  customText?: (e: CustomEmoji) => string
  onPick?: (emoji: PickedEmoji) => void
  disabled?: boolean
  className?: string
  size?: number
  title?: string
}) {
  const [open, setOpen] = useState(false)
  const wrapRef = useRef<HTMLSpanElement>(null)
  const close = useCallback(() => setOpen(false), [])

  function field(): Field | null {
    return target?.current ?? wrapRef.current?.parentElement?.querySelector<Field>(FIELD_SELECTOR) ?? null
  }

  function pick(p: PickedEmoji) {
    if (onPick) return onPick(p)
    const el = field()
    if (el && !el.disabled && !el.readOnly) insertIntoField(el, pickedText(p, customText))
  }

  return (
    <span className={`emoji-button-wrap ${className}`.trim()} ref={wrapRef}>
      <button
        type="button"
        className={`icon-button emoji-button${open ? ' open' : ''}`}
        title={title}
        aria-label={title}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        // Fokus und Auswahl im Textfeld behalten.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => setOpen((o) => !o)}
      >
        <Smile size={size} />
      </button>
      {open && <EmojiPicker onPick={pick} onClose={close} custom={custom} />}
    </span>
  )
}

/**
 * `<input>` mit Emoji-Button rechts im Feld (nur Unicode-Emojis). Nimmt alle Props eines `<input>`, also als
 * direkter Ersatz: `<input …/>` → `<EmojiInput …/>`.
 */
export function EmojiInput({
  className,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { ref?: RefObject<HTMLInputElement | null> }) {
  return (
    <span className="emoji-field">
      <input className={className} {...props} />
      <EmojiButton className="emoji-field-button" disabled={props.disabled || props.readOnly} size={16} />
    </span>
  )
}

/** Wie `EmojiInput`, für mehrzeilige Felder. */
export function EmojiTextarea({
  className,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { ref?: RefObject<HTMLTextAreaElement | null> }) {
  return (
    <span className="emoji-field emoji-field-area">
      <textarea className={className} {...props} />
      <EmojiButton className="emoji-field-button" disabled={props.disabled || props.readOnly} size={16} />
    </span>
  )
}
