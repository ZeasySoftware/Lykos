import { useMemo } from 'react'
import type { ReactNode } from 'react'
import { emojiOnlyCount } from './customEmoji'
import { renderInline } from './markdownInline'
import type { MarkdownOptions } from './markdownInline'
import { parseBlocks } from './markdownParser'
import type { Block } from './markdownParser'

export type { MarkdownOptions, ResolvedMention } from './markdownInline'

/**
 * Rendert Nachrichten-Markdown sicher als React-Elemente (kein `dangerouslySetInnerHTML`).
 * Syntax siehe `markdownParser.ts`.
 */
export function Markdown({ content, options }: { content: string; options?: MarkdownOptions }) {
  const blocks = useMemo(() => parseBlocks(content), [content])
  const jumbo = useMemo(() => !!options?.jumbo && emojiOnlyCount(content) > 0, [content, options?.jumbo])
  const opts = options ?? {}
  if (jumbo) return <span className="md-jumbo">{blocks.map((b, i) => renderBlock(b, i, opts, true))}</span>
  return <>{blocks.map((b, i) => renderBlock(b, i, opts))}</>
}

function renderBlock(block: Block, key: number, opts: MarkdownOptions, jumbo = false): ReactNode {
  switch (block.type) {
    case 'paragraph':
      return <span key={key}>{renderInline(block.children, opts, jumbo)}</span>
    case 'codeblock':
      return (
        <pre key={key} className="md-codeblock" data-lang={block.lang ?? undefined}>
          <code>{block.text}</code>
        </pre>
      )
    case 'quote':
      return (
        <blockquote key={key} className="md-quote">
          {block.children.map((b, i) => renderBlock(b, i, opts, jumbo))}
        </blockquote>
      )
  }
}
