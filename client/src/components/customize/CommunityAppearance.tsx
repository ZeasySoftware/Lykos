/**
 * Community-Einstellungen, Tab „Darstellung“ (Recht MANAGE_COMMUNITY): Icon, Banner, Beschreibung,
 * Akzentfarbe und Systemkanal – mit Live-Vorschau von Community-Leiste, Kanalleisten-Kopf und Einladung.
 */
import { useState } from 'react'
import { useUnsavedChanges } from '../../lib/unsaved'
import type { FormEvent } from 'react'
import { endpoints, uploadCommunityImage } from '../../lib/api'
import type { Community, CommunityDetail } from '../../lib/api'
import { IMAGE_FORMATS, accentStyle, prepareImage, sidebarHeaderClass } from '../../lib/customize'
import { errorText } from '../../lib/errors'
import { useStore } from '../../store'
import { InviteCard, SidebarHeaderBanner } from './CommunityView'
import { ColorPicker, CommunityIcon, ImagePickerRow } from './parts'
import { EmojiTextarea } from '../EmojiButton'
import { Hash } from 'lucide-react'

const DESCRIPTION_MAX = 300

type Feedback = { ok: boolean; text: string } | null
type ImageKind = 'icon' | 'banner'

export function CommunityAppearance({ detail }: { detail: CommunityDetail }) {
  const updateCommunity = useStore((s) => s.updateCommunity)
  const memberCount = useStore((s) => s.members[detail.community.id]?.length)
  const { community } = detail
  const [description, setDescription] = useState(community.description ?? '')
  const [accent, setAccent] = useState<number | null>(community.accent_color ?? null)
  const [systemChannel, setSystemChannel] = useState(community.system_channel_id ?? '')
  const [busy, setBusy] = useState<ImageKind | 'save' | null>(null)
  const [msg, setMsg] = useState<Feedback>(null)

  const dirty =
    description.trim() !== (community.description ?? '') ||
    accent !== (community.accent_color ?? null) ||
    systemChannel !== (community.system_channel_id ?? '')
  useUnsavedChanges({
    dirty,
    busy: busy === 'save',
    save,
    reset: () => {
      setDescription(community.description ?? '')
      setAccent(community.accent_color ?? null)
      setSystemChannel(community.system_channel_id ?? '')
      setMsg(null)
    },
  })

  const textChannels = detail.channels.filter((c) => c.kind === 'text').sort((a, b) => a.position - b.position)

  async function upload(kind: ImageKind, file: File) {
    setMsg(null)
    setBusy(kind)
    try {
      let prepared
      try {
        prepared =
          kind === 'icon'
            ? await prepareImage(file, IMAGE_FORMATS.communityIcon, 'icon', true)
            : await prepareImage(file, IMAGE_FORMATS.communityBanner, 'banner')
      } catch {
        setMsg({ ok: false, text: 'Das Bild konnte nicht gelesen werden (PNG, JPEG, WebP oder GIF).' })
        return
      }
      updateCommunity(await uploadCommunityImage(community.id, kind, prepared.blob, prepared.filename))
      setMsg({ ok: true, text: kind === 'icon' ? 'Icon aktualisiert.' : 'Banner aktualisiert.' })
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  async function remove(kind: ImageKind) {
    setMsg(null)
    setBusy(kind)
    try {
      updateCommunity(await endpoints.deleteCommunityImage(community.id, kind))
      setMsg({ ok: true, text: kind === 'icon' ? 'Icon entfernt.' : 'Banner entfernt.' })
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  async function save(e?: FormEvent) {
    e?.preventDefault()
    setMsg(null)
    setBusy('save')
    try {
      const updated = await endpoints.updateCommunity(community.id, {
        description: description.trim() || null,
        accent_color: accent,
        system_channel_id: systemChannel || null,
      })
      updateCommunity(updated)
      setMsg({ ok: true, text: 'Gespeichert.' })
    } catch (err) {
      setMsg({ ok: false, text: errorText(err) })
    } finally {
      setBusy(null)
    }
  }

  // Vorschau mit den noch nicht gespeicherten Angaben.
  const preview: Community = { ...community, description: description.trim() || null, accent_color: accent }

  return (
    <div className="appearance-layout">
      <form className="form" onSubmit={save}>

        <div className="field">
          <span>Icon</span>
          <div className="appearance-image-row">
            <CommunityIcon community={community} className="appearance-icon" />
            <ImagePickerRow
              label="Icon"
              hint="Wird quadratisch zugeschnitten und auf 256 px verkleinert; animierte GIFs bis 1 MB bleiben unverändert. Metadaten werden entfernt."
              hasImage={!!community.icon_url}
              busy={busy !== null}
              onFile={(f) => void upload('icon', f)}
              onRemove={() => void remove('icon')}
              onTooLarge={() => setMsg({ ok: false, text: 'Die Datei ist zu groß.' })}
            />
          </div>
        </div>

        <div className="field">
          <span>Banner</span>
          <ImagePickerRow
            label="Banner"
            hint="Erscheint oben in der Kanalleiste und auf der Einladungsseite. Zuschnitt 16:9 (960 × 540 px)."
            hasImage={!!community.banner_url}
            busy={busy !== null}
            onFile={(f) => void upload('banner', f)}
            onRemove={() => void remove('banner')}
            onTooLarge={() => setMsg({ ok: false, text: 'Die Datei ist zu groß.' })}
          />
        </div>

        <label className="field">
          <span>
            Beschreibung{' '}
            <span className="muted small">
              ({Array.from(description).length}/{DESCRIPTION_MAX})
            </span>
          </span>
          <EmojiTextarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            maxLength={DESCRIPTION_MAX}
            rows={3}
            placeholder="Worum geht es in dieser Community?"
          />
        </label>

        <div className="field">
          <span>Akzentfarbe</span>
          <ColorPicker value={accent} onChange={setAccent} label="Akzentfarbe" noneLabel="Keine Akzentfarbe" />
          <span className="muted small">Dezent genutzt: Verlauf ohne Banner und Auswahlmarkierung in der Leiste.</span>
        </div>

        <label className="field">
          <span>Systemkanal</span>
          <select value={systemChannel} onChange={(e) => setSystemChannel(e.target.value)}>
            <option value="">Keine Beitritts-Meldungen</option>
            {textChannels.map((c) => (
              <option key={c.id} value={c.id}>
                #{c.name}
              </option>
            ))}
          </select>
          <span className="muted small">Hier erscheint „… ist beigetreten“, wenn jemand der Community beitritt.</span>
        </label>

        {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
      </form>

      <aside className="appearance-preview" aria-label="Vorschau">
        <h4>Vorschau</h4>
        <div className="preview-shell">
          <div className="preview-rail">
            <span
              className={`rail-item active${accent != null ? ' has-accent' : ''}`}
              style={accentStyle(accent)}
            >
              <CommunityIcon community={community} />
            </span>
          </div>
          <div className="preview-sidebar">
            <div className={`sidebar-header${sidebarHeaderClass(preview)}`}>
              <SidebarHeaderBanner community={preview} />
              <span className="truncate">{community.name}</span>
            </div>
            <div className="preview-channels">
              {textChannels.slice(0, 3).map((c) => (
                <span key={c.id} className="channel">
                  <span className="channel-icon" aria-hidden="true">
                    <Hash size={18} />
                  </span>
                  <span className="truncate">{c.name}</span>
                </span>
              ))}
            </div>
          </div>
        </div>
        <InviteCard
          preview={{
            community: {
              id: community.id,
              name: community.name,
              icon_url: community.icon_url,
              banner_url: community.banner_url,
              description: preview.description,
              accent_color: accent,
            },
            member_count: memberCount ?? 1,
          }}
        />
      </aside>
    </div>
  )
}
