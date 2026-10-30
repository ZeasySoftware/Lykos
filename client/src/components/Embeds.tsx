import type { ReactNode } from 'react'
import type { Embed } from '../lib/api'
import { openExternalLink } from '../lib/external'
import { Markdown } from '../lib/markdown'
import type { MarkdownOptions } from '../lib/markdown'
import { roleColor } from '../lib/permissionLabels'

const timeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/** Externer Link (nur http/https, vom Server geprüft) – im Desktop über den Standardbrowser. */
function ExternalLink({ url, className, children }: { url: string; className?: string; children: ReactNode }) {
  return (
    <a className={className} href={url} target="_blank" rel="noopener noreferrer" onClick={(e) => void openExternalLink(e, url)}>
      {children}
    </a>
  )
}

/**
 * Karten aus Webhook- und Bot-Nachrichten (Discord-Embeds). Bilder gibt es bewusst nicht – der Server speichert
 * keine Bild-URLs, damit kein Client Inhalte von fremden Servern lädt.
 */
export function EmbedList({ embeds, markdown }: { embeds: Embed[] | undefined; markdown: MarkdownOptions }) {
  if (!embeds?.length) return null
  return (
    <div className="embeds">
      {embeds.map((embed, i) => (
        <EmbedCard key={i} embed={embed} markdown={markdown} />
      ))}
    </div>
  )
}

function EmbedCard({ embed, markdown }: { embed: Embed; markdown: MarkdownOptions }) {
  const color = embed.color != null ? roleColor(embed.color) : undefined
  const time = embed.timestamp ? new Date(embed.timestamp) : null
  return (
    <div className="embed" style={color ? { borderInlineStartColor: color } : undefined}>
      {embed.author && (
        <div className="embed-author">
          {embed.author.url ? <ExternalLink url={embed.author.url}>{embed.author.name}</ExternalLink> : embed.author.name}
        </div>
      )}
      {embed.title && (
        <div className="embed-title">
          {embed.url ? (
            <ExternalLink url={embed.url} className="md-link">
              {embed.title}
            </ExternalLink>
          ) : (
            embed.title
          )}
        </div>
      )}
      {embed.description && (
        <div className="embed-description">
          <Markdown content={embed.description} options={markdown} />
        </div>
      )}
      {!!embed.fields?.length && (
        <dl className="embed-fields">
          {embed.fields.map((f, i) => (
            <div key={i} className={`embed-field${f.inline ? ' inline' : ''}`}>
              <dt>{f.name}</dt>
              <dd>
                <Markdown content={f.value} options={markdown} />
              </dd>
            </div>
          ))}
        </dl>
      )}
      {(embed.footer || (time && !Number.isNaN(time.getTime()))) && (
        <div className="embed-footer">
          {embed.footer?.text}
          {embed.footer && time && ' · '}
          {time && !Number.isNaN(time.getTime()) && <time dateTime={embed.timestamp}>{timeFmt.format(time)}</time>}
        </div>
      )}
    </div>
  )
}
