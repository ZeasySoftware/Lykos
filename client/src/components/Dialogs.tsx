import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { ChannelKind, Invite, InvitePreview } from '../lib/api'
import { errorText } from '../lib/errors'
import { extractInviteCode } from '../lib/invite'
import { useStore } from '../store'
import { Modal } from './Modal'
import { EmojiInput } from './EmojiButton'
import { Check, Hash, Volume2 } from 'lucide-react'
import { InviteCard } from './customize/CommunityView'

/** Community anlegen oder per Einladung beitreten. */
export function AddCommunityDialog({ onClose }: { onClose: () => void }) {
  const createCommunity = useStore((s) => s.createCommunity)
  const acceptInvite = useStore((s) => s.acceptInvite)
  // Instanz-Einstellung: Communities anlegen ggf. nur für Instanz-Admins (Server prüft ebenso).
  const canCreate = useStore(
    (s) => s.instance?.allow_community_creation !== false || !!s.user?.is_instance_admin,
  )
  const [tab, setTab] = useState<'create' | 'join'>(canCreate ? 'create' : 'join')
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
          <label className="field">
            <span>Name der Community</span>
            <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} autoFocus required />
          </label>
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

export function CreateChannelDialog({
  communityId,
  initialKind = 'text',
  onClose,
}: {
  communityId: string
  initialKind?: ChannelKind
  onClose: () => void
}) {
  const loadDetail = useStore((s) => s.loadDetail)
  const selectChannel = useStore((s) => s.selectChannel)
  const [name, setName] = useState('')
  const [kind, setKind] = useState<ChannelKind>(initialKind)
  const [topic, setTopic] = useState('')
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

  async function accept() {
    setBusy(true)
    setError(null)
    try {
      await acceptInvite(code)
    } catch (err) {
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
            disabled={busy || !preview || preview.kind === 'instance'}
          >
            Beitreten
          </button>
        </div>
      </div>
    </Modal>
  )
}
