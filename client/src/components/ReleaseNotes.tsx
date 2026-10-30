import { useMemo } from 'react'
import type { ReactNode } from 'react'
import { openExternalLink } from '../lib/external'
import { renderInline } from '../lib/markdownInline'
import { parseReleaseNotes } from '../lib/releaseNotes'
import type { NoteBlock, NoteInline } from '../lib/releaseNotes'

/** Release-Notizen (Markdown aus dem Gitea-Release) sicher als React-Elemente. Syntax siehe `releaseNotes.ts`. */
export function ReleaseNotes({ text }: { text: string }) {
  const blocks = useMemo(() => parseReleaseNotes(text), [text])
  return <div className="release-notes">{blocks.map(renderBlock)}</div>
}

function renderBlock(block: NoteBlock, key: number): ReactNode {
  switch (block.type) {
    case 'heading': {
      const Tag = block.level === 1 ? 'h3' : block.level === 2 ? 'h4' : 'h5'
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
    n.type === 'named-link' ? (
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
