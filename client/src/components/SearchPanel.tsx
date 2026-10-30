import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { Message } from '../lib/api'
import { errorText } from '../lib/errors'
import { displayName, dmTitle, useStore } from '../store'
import { isMobileLayout } from '../lib/mobile'
import { Search, X } from 'lucide-react'

const PAGE = 25
const dateTimeFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

/**
 * Ergebnisliste der Nachrichtensuche (rechte Spalte). Sucht in der Community bzw. in allen eigenen DMs,
 * optional nur im aktuellen Kanal. Ein Klick springt zur Nachricht.
 * Wird pro Suche (Begriff, Bereich, Kanal) neu gemountet (`key`) und lädt dann die erste Seite.
 */
export function SearchPanel() {
  const search = useStore((s) => s.search)
  const openSearch = useStore((s) => s.openSearch)
  const closeSearch = useStore((s) => s.closeSearch)
  const jumpToMessage = useStore((s) => s.jumpToMessage)
  const currentChannel = useStore((s) => (s.dmView ? s.selectedDmId : s.selectedChannelId))
  const [query, setQuery] = useState(search?.query ?? '')
  const [results, setResults] = useState<Message[] | null>(null)
  const [hasMore, setHasMore] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const communityId = search?.communityId ?? null
  const channelId = search?.channelId ?? null
  const activeQuery = search?.query ?? ''

  function fetchPage(before?: string) {
    return endpoints.search(communityId, { q: activeQuery, channel_id: channelId ?? undefined, before, limit: PAGE })
  }

  // Erste Seite beim Öffnen (die Komponente wird pro Suche neu gemountet).
  useEffect(() => {
    let cancelled = false
    endpoints
      .search(communityId, { q: activeQuery, channel_id: channelId ?? undefined, limit: PAGE })
      .then((page) => {
        if (cancelled) return
        setResults(page)
        setHasMore(page.length === PAGE)
      })
      .catch((e) => !cancelled && setError(errorText(e)))
      .finally(() => !cancelled && setLoading(false))
    return () => {
      cancelled = true
    }
  }, [communityId, channelId, activeQuery])

  async function more(before: string) {
    setLoading(true)
    setError(null)
    try {
      const page = await fetchPage(before)
      setResults((prev) => [...(prev ?? []), ...page])
      setHasMore(page.length === PAGE)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setLoading(false)
    }
  }

  if (!search) return null

  function submit(e: FormEvent) {
    e.preventDefault()
    if (query.trim() && search) openSearch({ ...search, query: query.trim() })
  }

  function jump(m: Message) {
    jumpToMessage(m).then(
      // Mobil verdeckt die Trefferliste den Chat – nach dem Sprung schließen.
      () => isMobileLayout() && closeSearch(),
      (e) => setError(errorText(e)),
    )
  }

  return (
    <aside className="search-panel" aria-label="Suchergebnisse">
      <header className="search-panel-header">
        <form className="pane-search search-panel-form" role="search" onSubmit={submit}>
          <Search size={15} aria-hidden="true" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Suchbegriff"
            maxLength={200}
            autoFocus
          />
        </form>
        <button type="button" className="icon-button" title="Suche schließen" onClick={closeSearch}>
          <X size={18} />
        </button>
      </header>
      <div className="search-scope">
        <span className="muted small">{communityId ? 'In dieser Community' : 'In deinen Direktnachrichten'}</span>
        {currentChannel && (
          <label className="checkbox small">
            <input
              type="checkbox"
              checked={channelId !== null}
              onChange={(e) => openSearch({ ...search, channelId: e.target.checked ? currentChannel : null })}
            />
            Nur aktueller Kanal
          </label>
        )}
      </div>
      <div className="search-results">
        {error && <p className="error small">{error}</p>}
        {results?.length === 0 && !loading && <p className="muted small center">Keine Treffer.</p>}
        {results?.map((m) => (
          <SearchResult key={m.id} message={m} onClick={() => jump(m)} />
        ))}
        {loading && <p className="muted small center">Suche …</p>}
        {hasMore && !loading && results && (
          <button type="button" className="button search-more" onClick={() => void more(results[results.length - 1].id)}>
            Weitere Treffer
          </button>
        )}
      </div>
    </aside>
  )
}

function SearchResult({ message, onClick }: { message: Message; onClick: () => void }) {
  const where = useStore((s) => {
    if (message.community_id === null) {
      const dm = s.dms[message.channel_id]
      return dm ? dmTitle(dm, s.user?.id) : 'Direktnachricht'
    }
    const channel = s.details[message.community_id]?.channels.find((c) => c.id === message.channel_id)
    return channel ? `#${channel.name}` : ''
  })
  const files = message.attachments.length

  return (
    <button type="button" className="search-result" onClick={onClick}>
      <span className="search-result-meta">
        <strong className="truncate">{displayName(message.author)}</strong>
        <span className="muted small truncate">{where}</span>
        <time className="muted small" dateTime={message.created_at}>
          {dateTimeFmt.format(new Date(message.created_at))}
        </time>
      </span>
      <span className="search-result-content">
        {message.content || (files > 0 ? `${files} ${files === 1 ? 'Anhang' : 'Anhänge'}` : '')}
      </span>
    </button>
  )
}
