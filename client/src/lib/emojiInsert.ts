import type { CustomEmoji } from './api'
import { formatCustomToken } from './customEmoji'

/** Ergebnis des Emoji-Pickers (Typ aus `components/EmojiPicker`, hier strukturell, um Zyklen zu vermeiden). */
type Picked = { kind: 'unicode'; emoji: string } | { kind: 'custom'; emoji: CustomEmoji }

export type Field = HTMLInputElement | HTMLTextAreaElement

/**
 * Text an der Cursorposition in ein (auch von React kontrolliertes) Eingabefeld einfügen: Der Wert wird über den
 * nativen Setter gesetzt und ein `input`-Event ausgelöst, damit `onChange` des Felds wie beim Tippen läuft.
 * `maxLength` wird beachtet. Gibt zurück, ob eingefügt wurde.
 */
export function insertIntoField(el: Field, text: string): boolean {
  const start = el.selectionStart ?? el.value.length
  const end = el.selectionEnd ?? start
  const next = el.value.slice(0, start) + text + el.value.slice(end)
  if (el.maxLength > 0 && next.length > el.maxLength) return false
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
  Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(el, next)
  el.dispatchEvent(new Event('input', { bubbles: true }))
  const caret = start + text.length
  el.focus()
  el.setSelectionRange(caret, caret)
  return true
}

/** Text, den ein ausgewähltes Emoji in ein Feld einfügt (`customText` entscheidet bei Custom-Emotes). */
export function pickedText(p: Picked, customText?: (e: CustomEmoji) => string): string {
  if (p.kind === 'unicode') return p.emoji
  return customText ? customText(p.emoji) : formatCustomToken(p.emoji)
}
