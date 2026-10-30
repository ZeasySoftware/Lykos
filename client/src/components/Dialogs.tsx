import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError, endpoints } from '../lib/api'
import type { Category, ChannelKind, DiscordTemplatePreview, Invite, InvitePreview, TemplateChannel } from '../lib/api'
import { sortedCategories } from '../lib/categories'
import { extractTemplateCode } from '../lib/discordTemplate'
import { errorText } from '../lib/errors'
import { extractInviteCode } from '../lib/invite'
import { useStore } from '../store'
import { Modal } from './Modal'
import { EmojiInput } from './EmojiButton'
import { Check, ChevronRight, Hash, Import, Volume2 } from 'lucide-react'
import { InviteCard } from './customize/CommunityView'

/** Community anlegen (leer oder aus einer Discord-Vorlage) oder per Einladung beitreten. */
export function AddCommunityDialog({ onClose }: { onClose: () => void }) {
  const createCommunity = useStore((s) => s.createCommunity)
  const acceptInvite = useStore((s) => s.acceptInvite)
  // Instanz-Einstellung: Communities anlegen ggf. nur für Instanz-Admins (Server prüft ebenso).
  const canCreate = useStore(
    (s) => s.instance?.allow_community_creation !== false || !!s.user?.is_instance_admin,
  )
  const [tab, setTab] = useState<'create' | 'join'>(canCreate ? 'create' : 'join')
  const [fromTemplate, setFromTemplate] = useState(false)
  const [name, setName] = useState('')
  const [code, setCode] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setBusy(true)
    try {
      if (tab === 'create') await createCommunity(name.trim())
      else await acceptInvite(extractInviteCode(code))
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  if (tab === 'create' && fromTemplate) {
    return <DiscordTemplateDialog onBack={() => setFromTemplate(false)} onClose={onClose} />
  }

  return (
    <Modal title="Community hinzufügen" onClose={onClose}>
      <form onSubmit={submit} className="form">
        {canCreate ? (
          <div className="tabs" role="tablist">
            <button type="button" className={tab === 'create' ? 'active' : ''} onClick={() => setTab('create')}>
              Neu anlegen
            </button>
            <button type="button" className={tab === 'join' ? 'active' : ''} onClick={() => setTab('join')}>
              Beitreten
            </button>
          </div>
        ) : (
          <p className="muted small">Neue Communities legen auf dieser Instanz nur Admins an.</p>
        )}
        {tab === 'create' ? (
          <>
            <label className="field">
              <span>Name der Community</span>
              <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} autoFocus required />
            </label>
            <button type="button" className="template-import-link" onClick={() => setFromTemplate(true)}>
              <Import size={18} aria-hidden="true" />
              <span>
                <strong>Aus Discord-Vorlage importieren</strong>
                <span className="muted small">Rollen, Kategorien und Kanäle aus einem discord.new-Link übernehmen</span>
              </span>
              <ChevronRight size={16} aria-hidden="true" />
            </button>
          </>
        ) : (
          <label className="field">
            <span>Einladung</span>
            <input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="Code oder Einladungslink"
              autoFocus
              required
            />
          </label>
        )}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {tab === 'create' ? 'Anlegen' : 'Beitreten'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Community aus einer Discord-Servervorlage: Link einfügen → Vorschau (Name änderbar) → Anlegen. */
function DiscordTemplateDialog({ onBack, onClose }: { onBack: () => void; onClose: () => void }) {
  const createFromTemplate = useStore((s) => s.createCommunityFromTemplate)
  const [link, setLink] = useState('')
  const [preview, setPreview] = useState<DiscordTemplatePreview | null>(null)
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    if (!preview) {
      const code = extractTemplateCode(link)
      if (!code) {
        setError(errorText(new ApiError(400, 'invalid_template_code')))
        return
      }
      setBusy(true)
      try {
        const p = await endpoints.discordTemplate(code)
        setPreview(p)
        setName(p.name)
      } catch (err) {
        setError(errorText(err))
      } finally {
        setBusy(false)
      }
      return
    }
    setBusy(true)
    try {
      await createFromTemplate(preview.code, name.trim())
      onClose()
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  const back = () => {
    setError(null)
    if (preview) setPreview(null)
    else onBack()
  }

  return (
    <Modal title="Aus Discord-Vorlage importieren" onClose={onClose}>
      <form onSubmit={submit} className="form">
        {preview ? (
          <>
            <label className="field">
              <span>Name der Community</span>
              <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} autoFocus required />
            </label>
            {preview.description && <p className="muted small">{preview.description}</p>}
            <TemplateStructure preview={preview} />
            <ul className="template-notes">
              <li>Nachrichten, Mitglieder, Emotes und Bots werden nicht übernommen.</li>
              {preview.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          </>
        ) : (
          <>
            <label className="field">
              <span>Vorlagen-Link</span>
              <input
                value={link}
                onChange={(e) => setLink(e.target.value)}
                placeholder="https://discord.new/…"
                autoFocus
                required
                inputMode="url"
                spellCheck={false}
              />
            </label>
            <p className="muted small">
              Den Link erzeugst du in Discord unter Server-Einstellungen → Servervorlage. Lykos ruft die Vorlage über
              die öffentliche Discord-Schnittstelle ab; dabei wird nur der Vorlagen-Code übertragen.
            </p>
          </>
        )}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <div className="form-actions-left">
            <button type="button" className="button ghost" onClick={back} disabled={busy}>
              Zurück
            </button>
          </div>
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {preview ? (busy ? 'Lege an …' : 'Anlegen') : busy ? 'Lade …' : 'Vorschau'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Übersicht der Vorlage: Zahlen und die Kanalstruktur wie in der Seitenleiste. */
function TemplateStructure({ preview }: { preview: DiscordTemplatePreview }) {
  const { counts } = preview
  const channelItem = (c: TemplateChannel, i: number) => (
    <li key={i}>
      {c.kind === 'text' ? <Hash size={14} aria-hidden="true" /> : <Volume2 size={14} aria-hidden="true" />}
      <span className="truncate">{c.name}</span>
      {c.from_forum && <span className="muted small">Forum</span>}
    </li>
  )
  return (
    <section className="template-preview" aria-label="Struktur der Vorlage">
      <p className="template-counts">
        {counts.roles} Rollen · {counts.categories} Kategorien · {counts.text_channels} Textkanäle ·{' '}
        {counts.voice_channels} Sprachkanäle
      </p>
      <div className="template-tree">
        {preview.channels.length > 0 && <ul>{preview.channels.map(channelItem)}</ul>}
        {preview.categories.map((cat, i) => (
          <div key={i}>
            <h4>{cat.name}</h4>
            {cat.channels.length > 0 ? (
              <ul>{cat.channels.map(channelItem)}</ul>
            ) : (
              <p className="muted small">Keine Kanäle</p>
            )}
          </div>
        ))}
      </div>
    </section>
  )
}

/** Auswahl der Kategorie (leer = ohne Kategorie). */
export function CategorySelect({
  categories,
  value,
  onChange,
}: {
  categories: Category[]
  value: string
  onChange: (id: string) => void
}) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">Ohne Kategorie</option>
      {categories.map((c) => (
        <option key={c.id} value={c.id}>
          {c.name}
        </option>
      ))}
    </select>
  )
}

/** Kategorie anlegen oder (mit `categoryId`) umbenennen. */
export function CategoryDialog({
  communityId,
  categoryId,
  onClose,
}: {
  communityId: string
  categoryId?: string
  onClose: () => void
}) {
  const existing = useStore((s) => s.details[communityId]?.categories?.find((c) => c.id === categoryId))
  const [name, setName] = useState(existing?.name ?? '')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setBusy(true)
    try {
      // Die Seitenleiste aktualisiert das Gateway-Event (CATEGORY_CREATE/UPDATE).
      if (categoryId) await endpoints.updateCategory(categoryId, { name: name.trim() })
      else await endpoints.createCategory(communityId, name.trim())
      onClose()
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <Modal title={categoryId ? 'Kategorie umbenennen' : 'Kategorie erstellen'} onClose={onClose}>
      <form onSubmit={submit} className="form">
        <label className="field">
          <span>Name der Kategorie</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={64} autoFocus required />
        </label>
        {!categoryId && (
          <p className="muted small">
            Kategorien gruppieren Kanäle in der Seitenleiste. Wer eine Kategorie sieht, richtet sich nach den Kanälen
            darin.
          </p>
        )}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {categoryId ? 'Speichern' : 'Erstellen'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

export function CreateChannelDialog({
  communityId,
  initialKind = 'text',
  initialCategory,
  onClose,
}: {
  communityId: string
  initialKind?: ChannelKind
  /** Vorauswahl der Kategorie („Kanal hier erstellen“). */
  initialCategory?: string | null
  onClose: () => void
}) {
  const loadDetail = useStore((s) => s.loadDetail)
  const selectChannel = useStore((s) => s.selectChannel)
  const categoryList = useStore((s) => s.details[communityId]?.categories)
  const categories = sortedCategories(categoryList)
  const [name, setName] = useState('')
  const [kind, setKind] = useState<ChannelKind>(initialKind)
  const [topic, setTopic] = useState('')
  const [category, setCategory] = useState(initialCategory ?? '')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setBusy(true)
    try {
      const channel = await endpoints.createChannel(communityId, {
        name: name.trim(),
        kind,
        topic: topic.trim() || undefined,
        category_id: category || undefined,
      })
      await loadDetail(communityId)
      if (channel.kind === 'text') selectChannel(channel.id)
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal title="Kanal anlegen" onClose={onClose}>
      <form onSubmit={submit} className="form">
        <fieldset className="field kind-cards">
          <legend>Kanaltyp</legend>
          <label className={`kind-card${kind === 'text' ? ' active' : ''}`}>
            <Hash size={22} aria-hidden="true" />
            <span>
              <strong>Text</strong>
              <span className="muted small">Nachrichten, Bilder, Dateien und Reaktionen</span>
            </span>
            <input type="radio" name="channel-kind" checked={kind === 'text'} onChange={() => setKind('text')} />
          </label>
          <label className={`kind-card${kind === 'voice' ? ' active' : ''}`}>
            <Volume2 size={22} aria-hidden="true" />
            <span>
              <strong>Sprache</strong>
              <span className="muted small">Gemeinsam sprechen, Bildschirm teilen und Kamera</span>
            </span>
            <input type="radio" name="channel-kind" checked={kind === 'voice'} onChange={() => setKind('voice')} />
          </label>
        </fieldset>
        <label className="field">
          <span>Name</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={64} autoFocus required />
        </label>
        <label className="field">
          <span>
            Thema <em className="muted">(optional)</em>
          </span>
          <EmojiInput value={topic} onChange={(e) => setTopic(e.target.value)} maxLength={1024} />
        </label>
        {categories.length > 0 && (
          <label className="field">
            <span>Kategorie</span>
            <CategorySelect categories={categories} value={category} onChange={setCategory} />
          </label>
        )}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            Anlegen
          </button>
        </div>
      </form>
    </Modal>
  )
}

const USES = [
  { label: 'Unbegrenzt', value: 0 },
  { label: '1 Nutzung', value: 1 },
  { label: '5 Nutzungen', value: 5 },
  { label: '10 Nutzungen', value: 10 },
  { label: '25 Nutzungen', value: 25 },
]
const EXPIRY = [
  { label: '1 Stunde', value: 1 },
  { label: '24 Stunden', value: 24 },
  { label: '7 Tage', value: 168 },
  { label: 'Nie', value: 0 },
]

export function InviteDialog({ communityId, onClose }: { communityId: string; onClose: () => void }) {
  const publicUrl = useStore((s) => s.instance?.public_url) ?? location.origin
  const [maxUses, setMaxUses] = useState(0)
  const [expires, setExpires] = useState(168)
  const [invite, setInvite] = useState<Invite | null>(null)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function create() {
    setError(null)
    setCopied(false)
    try {
      setInvite(
        await endpoints.createInvite(communityId, {
          max_uses: maxUses || undefined,
          expires_in_hours: expires || undefined,
        }),
      )
    } catch (err) {
      setError(errorText(err))
    }
  }

  const link = invite ? `${publicUrl.replace(/\/$/, '')}/invite/${invite.code}` : ''

  async function copy() {
    try {
      await navigator.clipboard.writeText(link)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <Modal title="Leute einladen" onClose={onClose}>
      <div className="form">
        <div className="field-row">
          <label className="field">
            <span>Gültig für</span>
            <select value={expires} onChange={(e) => setExpires(Number(e.target.value))}>
              {EXPIRY.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Nutzungen</span>
            <select value={maxUses} onChange={(e) => setMaxUses(Number(e.target.value))}>
              {USES.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <button type="button" className="button primary" onClick={create}>
          {invite ? 'Neuen Link erstellen' : 'Link erstellen'}
        </button>
        {invite && (
          <div className="copy-row">
            <input readOnly value={link} onFocus={(e) => e.currentTarget.select()} />
            <button type="button" className="button" onClick={copy}>
              {copied ? (
                <>
                  <Check size={14} /> Kopiert
                </>
              ) : (
                'Kopieren'
              )}
            </button>
          </div>
        )}
        {error && <p className="error">{error}</p>}
      </div>
    </Modal>
  )
}

/** Wird angezeigt, wenn ein angemeldeter Nutzer einen Einladungslink öffnet. */
export function AcceptInviteDialog({ code }: { code: string }) {
  const acceptInvite = useStore((s) => s.acceptInvite)
  const clearPendingInvite = useStore((s) => s.clearPendingInvite)
  const [preview, setPreview] = useState<InvitePreview | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    endpoints
      .invite(code)
      .then((p) => !cancelled && setPreview(p))
      .catch((e) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [code])

  const [banned, setBanned] = useState(false)

  async function accept() {
    setBusy(true)
    setError(null)
    try {
      await acceptInvite(code)
    } catch (err) {
      // Gebannt: verständliche Meldung, erneutes Beitreten bringt nichts.
      setBanned(err instanceof ApiError && err.code === 'banned')
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <Modal title="Einladung" onClose={clearPendingInvite}>
      <div className="form">
        {preview ? (
          <>
            <InviteCard preview={preview} />
            {preview.kind === 'instance' && (
              <p className="muted small">Diese Einladung gilt für neue Konten – du bist bereits registriert.</p>
            )}
          </>
        ) : (
          !error && <p className="muted">Lade Einladung …</p>
        )}
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={clearPendingInvite}>
            Später
          </button>
          <button
            type="button"
            className="button primary"
            onClick={accept}
            disabled={busy || banned || !preview || preview.kind === 'instance'}
          >
            Beitreten
          </button>
        </div>
      </div>
    </Modal>
  )
}
