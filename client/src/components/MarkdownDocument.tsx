import { useMemo } from 'react'
import type { ReactNode } from 'react'
import { openExternalLink } from '../lib/external'
import { renderInline } from '../lib/markdownInline'
import { parseReleaseNotes } from '../lib/releaseNotes'
import type { NoteBlock, NoteInline } from '../lib/releaseNotes'

const HEADINGS = ['h2', 'h3', 'h4', 'h5'] as const

/**
 * Einfaches Markdown (Release-Notizen, Rechtstexte der Instanz) sicher als React-Elemente – kein HTML,
 * Syntax siehe `releaseNotes.ts`. `topHeading` = Ebene, auf der `#` landet (die Seite hat darüber ihre
 * eigene Überschrift).
 */
export function MarkdownDocument({
  text,
  className = 'release-notes',
  topHeading = 3,
  keepLineBreaks = false,
}: {
  text: string
  className?: string
  topHeading?: 2 | 3
  keepLineBreaks?: boolean
}) {
  const blocks = useMemo(() => parseReleaseNotes(text, { keepLineBreaks }), [text, keepLineBreaks])
  return <div className={className}>{blocks.map((block, i) => renderBlock(block, i, topHeading))}</div>
}

function renderBlock(block: NoteBlock, key: number, topHeading: 2 | 3): ReactNode {
  switch (block.type) {
    case 'heading': {
      const Tag = HEADINGS[topHeading - 2 + block.level - 1]
      return <Tag key={key}>{renderNoteInline(block.children)}</Tag>
    }
    case 'list':
      return (
        <ul key={key}>
          {block.items.map((item, i) => (
            <li key={i}>{renderNoteInline(item)}</li>
          ))}
        </ul>
      )
    case 'paragraph':
      return <p key={key}>{renderNoteInline(block.children)}</p>
  }
}

function renderNoteInline(nodes: NoteInline[]): ReactNode[] {
  return nodes.map((n, i) =>
    n.type === 'break' ? (
      <br key={i} />
    ) : n.type === 'named-link' ? (
      <a
        key={i}
        className="md-link"
        href={n.url}
        target="_blank"
        rel="noopener noreferrer"
        onClick={(e) => void openExternalLink(e, n.url)}
      >
        {renderInline(n.children, {})}
      </a>
    ) : (
      <span key={i}>{renderInline([n], {})}</span>
    ),
  )
}
