import { useCallback, useMemo, useRef, useState } from 'react'
import type { KeyboardEvent, ReactNode, RefObject } from 'react'
import type { CustomEmoji } from '../lib/api'
import {
  completedShortcode,
  emojiQuery,
  formatCustomToken,
  replaceShortcodes,
  shortenCustomTokens,
} from '../lib/customEmoji'
import { useCustomEmojiList, useEmojiData, useEmojiPrefs } from '../lib/emojiData'
import { applySkin, findShortcode, shortcodeSuggestions } from '../lib/emojiSearch'
import type { ShortcodeSuggestion, UnicodeEmoji } from '../lib/emojiSearch'
import { useStore } from '../store'
import { CustomEmojiImage } from './CustomEmoji'

/**
 * Custom-Emotes im Composer: Im Textfeld stehen sie lesbar als `:name:`, beim Senden werden sie zu `<:name:id>`.
 * Gibt es denselben Namen in mehreren eigenen Communities, wird direkt das Token eingefügt (eindeutig).
 * Emotes der aktuellen Community haben bei gleichem Namen Vorrang.
 */
export function useComposerEmojis(communityId: string | null) {
  const all = useCustomEmojiList()
  const unicode = useEmojiData(false)
  const customs = useMemo(
    () => [...all].sort((a, b) => Number(b.community_id === communityId) - Number(a.community_id === communityId)),
    [all, communityId],
  )
  const counts = useMemo(() => {
    const m = new Map<string, number>()
    for (const e of all) m.set(e.name.toLowerCase(), (m.get(e.name.toLowerCase()) ?? 0) + 1)
    return m
  }, [all])

  /** Einfügetext für ein Custom-Emote im Composer. */
  const insertText = useCallback(
    (e: CustomEmoji) => ((counts.get(e.name.toLowerCase()) ?? 0) > 1 ? formatCustomToken(e) : `:${e.name}:`),
    [counts],
  )

  /** Vor dem Senden: `:name:` → Token bzw. Unicode-Emoji. */
  const encode = useCallback(
    (text: string) =>
      replaceShortcodes(text, (code) => {
        const hit = findShortcode(unicode, customs, code)
        if (!hit) return null
        return hit.kind === 'custom' ? formatCustomToken(hit.emoji) : hit.emoji.emoji
      }),
    [unicode, customs],
  )

  /** Zum Bearbeiten: eigene, eindeutige Tokens wieder als `:name:` zeigen. */
  const decode = useCallback(
    (text: string) =>
      shortenCustomTokens(text, (t) => {
        const e = customs.find((c) => c.id === t.id)
        return e && (counts.get(e.name.toLowerCase()) ?? 0) === 1 ? e.name : null
      }),
    [customs, counts],
  )

  return { customs, insertText, encode, decode }
}

/**
 * `:shortcode:`-Autovervollständigung (ab 2 Zeichen nach `:`), analog zur `@`-Vervollständigung:
 * Pfeiltasten wählen, Enter/Tab übernimmt, Esc schließt. Ein vollständig getipptes `:code:` eines Unicode-Emojis
 * wird sofort ersetzt.
 */
export function useEmojiAutocomplete({
  textRef,
  value,
  setValue,
  customs,
  insertText,
}: {
  textRef: RefObject<HTMLTextAreaElement | null>
  value: string
  setValue: (v: string) => void
  customs: CustomEmoji[]
  insertText: (e: CustomEmoji) => string
}) {
  const [query, setQuery] = useState<{ start: number; query: string } | null>(null)
  const [selected, setSelected] = useState(0)
  const unicode = useEmojiData(query !== null)
  const skin = useEmojiPrefs((s) => s.skin)
  const addRecent = useEmojiPrefs((s) => s.addRecent)
  const communities = useStore((s) => s.communities)
  const prevLength = useRef(value.length)

  const suggestions = useMemo(
    () => (query ? shortcodeSuggestions(unicode, customs, query.query) : []),
    [query, unicode, customs],
  )

  const setCaret = useCallback(
    (pos: number) =>
      requestAnimationFrame(() => {
        textRef.current?.focus()
        textRef.current?.setSelectionRange(pos, pos)
      }),
    [textRef],
  )

  function set(next: string) {
    prevLength.current = next.length
    setValue(next)
  }

  function replaceRange(start: number, end: number, text: string) {
    set(value.slice(0, start) + text + value.slice(end))
    setQuery(null)
    setCaret(start + text.length)
  }

  function unicodeText(e: UnicodeEmoji) {
    return applySkin(e, skin)
  }

  /** Nach jeder Eingabe bzw. Cursorbewegung aufrufen. */
  function update(next: string, caret: number) {
    const typed = next.length > prevLength.current
    prevLength.current = next.length
    if (typed && next[caret - 1] === ':') {
      const done = completedShortcode(next, caret)
      // Custom-Emotes bleiben als `:name:` stehen (werden beim Senden zum Token) und haben Vorrang.
      const hit = done && findShortcode(unicode, customs, done.code)
      if (done && hit && hit.kind === 'unicode') {
        addRecent(hit.emoji.emoji)
        const text = unicodeText(hit.emoji)
        set(next.slice(0, done.start) + text + next.slice(caret))
        setQuery(null)
        setCaret(done.start + text.length)
        return
      }
    }
    const q = emojiQuery(next, caret)
    if (q?.query !== query?.query) setSelected(0)
    setQuery(q)
  }

  function accept(s: ShortcodeSuggestion<CustomEmoji>) {
    if (!query) return
    const caret = textRef.current?.selectionStart ?? value.length
    if (s.kind === 'custom') {
      addRecent(`custom:${s.emoji.id}`)
      replaceRange(query.start, caret, `${insertText(s.emoji)} `)
    } else {
      addRecent(s.emoji.emoji)
      replaceRange(query.start, caret, `${unicodeText(s.emoji)} `)
    }
  }

  /** Tastendruck behandeln; `true` = verbraucht. */
  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>): boolean {
    if (suggestions.length === 0) return false
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const delta = e.key === 'ArrowDown' ? 1 : -1
      setSelected((i) => (i + delta + suggestions.length) % suggestions.length)
      return true
    }
    if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
      e.preventDefault()
      accept(suggestions[Math.min(selected, suggestions.length - 1)])
      return true
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      setQuery(null)
      return true
    }
    return false
  }

  const close = useCallback(() => setQuery(null), [])

  const list: ReactNode =
    suggestions.length > 0 ? (
      <ul className="mention-suggestions emoji-suggestions" role="listbox" aria-label="Emoji einfügen">
        {suggestions.map((s, i) => {
          const key = s.kind === 'custom' ? `c:${s.emoji.id}` : `u:${s.emoji.emoji}`
          const code = s.kind === 'custom' ? s.emoji.name : s.code
          const detail =
            s.kind === 'custom'
              ? (communities.find((c) => c.id === s.emoji.community_id)?.name ?? 'Custom-Emoji')
              : s.emoji.name
          return (
            <li key={key} role="option" aria-selected={i === selected}>
              <button
                type="button"
                className={i === selected ? 'active' : ''}
                // mousedown statt click, damit das Textfeld den Fokus behält.
                onMouseDown={(e) => {
                  e.preventDefault()
                  accept(s)
                }}
                onMouseEnter={() => setSelected(i)}
              >
                <span className="emoji-suggestion-glyph">
                  {s.kind === 'custom' ? (
                    <CustomEmojiImage id={s.emoji.id} name={s.emoji.name} animated={s.emoji.animated} size={22} tooltip={false} />
                  ) : (
                    unicodeText(s.emoji)
                  )}
                </span>
                <strong>:{code}:</strong>
                <span className="muted small truncate">{detail}</span>
              </button>
            </li>
          )
        })}
      </ul>
    ) : null

  return { update, onKeyDown, close, list, open: suggestions.length > 0 }
}
