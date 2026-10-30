/**
 * Einstellungen, Tab „Profil“: Avatar, Anzeigename, Pronomen, Status-Text, Über mich, Profilbanner bzw.
 * Bannerfarbe und Akzentfarbe – mit Live-Vorschau der Profilkarte neben dem Formular.
 */
import { useState } from 'react'
import { useUnsavedChanges } from '../../lib/unsaved'
import type { FormEvent } from 'react'
import { endpoints, uploadAvatar, uploadProfileBanner } from '../../lib/api'
import type { User } from '../../lib/api'
import { AVATAR_MAX_INPUT_BYTES, prepareAvatar } from '../../lib/avatar'
import { IMAGE_FORMATS, prepareImage } from '../../lib/customize'
import { errorText } from '../../lib/errors'
import { statusOf } from '../../lib/presence'
import { displayName, useStore } from '../../store'
import { Avatar } from '../Presence'
import { ProfileCardPreview } from './ProfileView'
import { ColorPicker, ImagePickerRow } from './parts'
import { EmojiInput, EmojiTextarea } from '../EmojiButton'

const BIO_MAX = 190
const STATUS_MAX = 128
const PRONOUNS_MAX = 40

type Feedback = { ok: boolean; text: string } | null

export function ProfileAppearance() {
  const user = useStore((s) => s.user)
  const setUser = useStore((s) => s.setUser)
  const status = useStore((s) => (s.user ? statusOf(s, s.user.id) : 'offline'))
  const [name, setName] = useState(user?.display_name ?? '')
  const [pronouns, setPronouns] = useState(user?.pronouns ?? '')
  const [statusText, setStatusText] = useState(user?.status_text ?? '')
  const [bio, setBio] = useState(user?.bio ?? '')
  const [bannerColor, setBannerColor] = useState<number | null>(user?.banner_color ?? null)
  const [accent, setAccent] = useState<number | null>(user?.accent_color ?? null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<Feedback>(null)

  if (!user) return null

  /** Bild aufbereiten und hochladen (Avatar oder Banner). */
  async function uploadImage(kind: 'avatar' | 'banner', file: File) {
    setMsg(null)
    if (kind === 'avatar' && file.size > AVATAR_MAX_INPUT_BYTES) {
      setMsg({ ok: false, text: 'Die Datei ist zu groß.' })
      return
    }
    setBusy(true)
    try {
      let prepared
      try {
        prepared =
          kind === 'avatar'
            ? await prepareAvatar(file)
            : await prepareImage(file, IMAGE_FORMATS.profileBanner, 'banner')
      } catch {
        setMsg({ ok: false, text: 'Das Bild konnte nicht gelesen werden (PNG, JPEG, WebP oder GIF).' })
        return
      }
      const updated =
        kind === 'avatar'
          ? await uploadAvatar(prepared.blob, prepared.filename)
          : await uploadProfileBanner(prepared.blob, prepared.filename)
      setUser(updated)
      setMsg({ ok: true, text: kind === 'avatar' ? 'Profilbild aktualisiert.' : 'Banner aktualisiert.' })
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) })
    } finally {
      setBusy(false)
    }
  }

  async function removeImage(kind: 'avatar' | 'banner') {
    setMsg(null)
    setBusy(true)
    try {
      setUser(kind === 'avatar' ? await endpoints.deleteAvatar() : await endpoints.deleteProfileBanner())
      setMsg({ ok: true, text: kind === 'avatar' ? 'Profilbild entfernt.' : 'Banner entfernt.' })
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) })
    } finally {
      setBusy(false)
    }
  }

  async function save(e?: FormEvent) {
    e?.preventDefault()
    setMsg(null)
    setBusy(true)
    try {
      setUser(
        await endpoints.updateMe({
          display_name: name.trim() || null,
          pronouns: pronouns.trim() || null,
          status_text: statusText.trim() || null,
          bio: bio.trim() || null,
          banner_color: bannerColor,
          accent_color: accent,
        }),
      )
      setMsg(null)
    } catch (err) {
      setMsg({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }

  // Ungespeicherte Änderungen → Leiste der Einstellungsebene (Bilder werden sofort hochgeladen).
  const dirty =
    name.trim() !== (user.display_name ?? '') ||
    pronouns.trim() !== (user.pronouns ?? '') ||
    statusText.trim() !== (user.status_text ?? '') ||
    bio.trim() !== (user.bio ?? '') ||
    bannerColor !== (user.banner_color ?? null) ||
    accent !== (user.accent_color ?? null)

  function reset() {
    if (!user) return
    setName(user.display_name ?? '')
    setPronouns(user.pronouns ?? '')
    setStatusText(user.status_text ?? '')
    setBio(user.bio ?? '')
    setBannerColor(user.banner_color ?? null)
    setAccent(user.accent_color ?? null)
    setMsg(null)
  }

  // Vorschau mit den noch nicht gespeicherten Angaben.
  const preview: User = {
    ...user,
    display_name: name.trim() || null,
    pronouns: pronouns.trim() || null,
    status_text: statusText.trim() || null,
    bio: bio.trim() || null,
    banner_color: bannerColor,
    accent_color: accent,
  }

  return (
    <div className="appearance-layout">
      <UnsavedReporter dirty={dirty} busy={busy} save={save} reset={reset} />
      <form className="form" onSubmit={save}>
        <div className="field">
          <span>Profilbild</span>
          <div className="appearance-image-row">
            <Avatar user={user} name={displayName(user)} size={64} />
            <ImagePickerRow
              label="Bild"
              hint="Wird quadratisch zugeschnitten und auf 256 px verkleinert; Metadaten (z. B. Standort) werden entfernt."
              hasImage={!!user.avatar_url}
              busy={busy}
              onFile={(f) => void uploadImage('avatar', f)}
              onRemove={() => void removeImage('avatar')}
              onTooLarge={() => setMsg({ ok: false, text: 'Die Datei ist zu groß.' })}
            />
          </div>
        </div>

        <div className="field">
          <span>Profilbanner</span>
          <ImagePickerRow
            label="Banner"
            hint="Zuschnitt 5:2 (600 × 240 px). Ohne Bild wird die Bannerfarbe gezeigt."
            hasImage={!!user.banner_url}
            busy={busy}
            onFile={(f) => void uploadImage('banner', f)}
            onRemove={() => void removeImage('banner')}
            onTooLarge={() => setMsg({ ok: false, text: 'Die Datei ist zu groß.' })}
          />
        </div>

        {!user.banner_url && (
          <div className="field">
            <span>Bannerfarbe</span>
            <ColorPicker value={bannerColor} onChange={setBannerColor} label="Bannerfarbe" noneLabel="Standard-Verlauf" />
          </div>
        )}

        <div className="field">
          <span>Akzentfarbe</span>
          <ColorPicker value={accent} onChange={setAccent} label="Akzentfarbe" noneLabel="Keine Akzentfarbe" />
          <span className="muted small">Rahmen und Verlauf deiner Profilkarte.</span>
        </div>

        <label className="field">
          <span>Benutzername</span>
          <input value={user.username} disabled />
        </label>
        <label className="field">
          <span>Anzeigename</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={64} placeholder={user.username} />
        </label>
        <label className="field">
          <span>
            Pronomen <em className="muted">(optional)</em>
          </span>
          <input
            value={pronouns}
            onChange={(e) => setPronouns(e.target.value)}
            maxLength={PRONOUNS_MAX}
            placeholder="z. B. sie/ihr, er/ihm, they/them"
          />
        </label>
        <label className="field">
          <span>Status-Text</span>
          <EmojiInput
            value={statusText}
            onChange={(e) => setStatusText(e.target.value)}
            maxLength={STATUS_MAX}
            placeholder="z. B. „Im Urlaub bis Montag“"
          />
        </label>
        <label className="field">
          <span>
            Über mich{' '}
            <span className="muted small">
              ({Array.from(bio).length}/{BIO_MAX})
            </span>
          </span>
          <EmojiTextarea
            value={bio}
            onChange={(e) => setBio(e.target.value)}
            maxLength={BIO_MAX}
            rows={3}
            placeholder="Ein paar Worte über dich – **fett**, *kursiv* und Links gehen auch."
          />
        </label>
        <p className="muted small">
          Profilbild, Banner, Status-Text und „Über mich“ sehen nur Mitglieder der Communities, in denen du bist, und
          deine DM-Partner.
        </p>
        {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
      </form>

      <aside className="appearance-preview" aria-label="Vorschau">
        <h4>Vorschau</h4>
        <ProfileCardPreview profile={preview} status={status} />
      </aside>
    </div>
  )
}

/** Meldet den Formularzustand an die Einstellungsebene (Hook außerhalb des frühen `return`). */
function UnsavedReporter(p: { dirty: boolean; busy: boolean; save: () => unknown; reset: () => void }) {
  useUnsavedChanges(p)
  return null
}
