import type { ReactNode } from 'react'
import { openExternalLink } from './external'
import { CustomEmojiImage } from '../components/CustomEmoji'
import { Spoiler } from '../components/Spoiler'
import type { Inline } from './markdownParser'

// Inline-Teil des Markdown-Renderers (Nachrichten und Release-Notizen); `Markdown` selbst liegt in markdown.tsx.

/** Aufgelöste Erwähnung: Anzeigename und ob sie den aktuellen Nutzer meint. */
export interface ResolvedMention {
  label: string
  self: boolean
}

export interface MarkdownOptions {
  /** Benutzername (wie geschrieben, ohne `@`) → Erwähnung; `null` = kein bekanntes Mitglied. */
  resolveMention?: (name: string) => ResolvedMention | null
  /** Hat der Server `@everyone` als Erwähnung gewertet? Sonst bleibt es normaler Text. */
  everyone?: boolean
  /** Reine Emoji-Nachrichten (bis 27 Emojis) groß darstellen – in der Nachrichtenliste, nicht in Vorschauen. */
  jumbo?: boolean
}

/** Inline-Knoten als React-Elemente (Nachrichten und Release-Notizen, siehe `ReleaseNotes.tsx`). */
export function renderInline(nodes: Inline[], opts: MarkdownOptions, jumbo = false): ReactNode[] {
  return nodes.map((n, i) => {
    switch (n.type) {
      case 'emoji':
        return <CustomEmojiImage key={i} id={n.id} name={n.name} animated={n.animated} size={jumbo ? 48 : 22} />
      case 'text':
        return n.text
      case 'bold':
        return <strong key={i}>{renderInline(n.children, opts, jumbo)}</strong>
      case 'italic':
        return <em key={i}>{renderInline(n.children, opts, jumbo)}</em>
      case 'strike':
        return <s key={i}>{renderInline(n.children, opts, jumbo)}</s>
      case 'spoiler':
        return <Spoiler key={i}>{renderInline(n.children, opts, jumbo)}</Spoiler>
      case 'code':
        return (
          <code key={i} className="md-code">
            {n.text}
          </code>
        )
      case 'link':
        return (
          <a
            key={i}
            className="md-link"
            href={n.url}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(e) => void openExternalLink(e, n.url)}
          >
            {n.url}
          </a>
        )
      case 'everyone':
        return opts.everyone ? (
          <span key={i} className="mention self">
            @everyone
          </span>
        ) : (
          '@everyone'
        )
      case 'mention':
        return renderMention(n.name, i, opts)
    }
  })
}

/** `@bob.` → Erwähnung von `bob` plus `.`, wenn nur der gekürzte Name ein Mitglied ist (wie auf dem Server). */
function renderMention(name: string, key: number, opts: MarkdownOptions): ReactNode {
  const trimmed = name.replace(/[.-]+$/, '')
  for (const candidate of [name, trimmed]) {
    const resolved = candidate.length >= 2 ? opts.resolveMention?.(candidate) : null
    if (resolved) {
      return (
        <span key={key}>
          <span className={`mention${resolved.self ? ' self' : ''}`} title={`@${candidate}`}>
            @{resolved.label}
          </span>
          {name.slice(candidate.length)}
        </span>
      )
    }
  }
  return `@${name}`
}
