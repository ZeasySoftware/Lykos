import { useCallback, useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints, uploadBotAvatar } from '../../lib/api'
import type { Bot, BotWithToken } from '../../lib/api'
import { AVATAR_MAX_INPUT_BYTES, prepareAvatar } from '../../lib/avatar'
import { confirmAction } from '../../lib/dialogs'
import { errorText } from '../../lib/errors'
import { displayName, useStore } from '../../store'
import { CopyButton } from '../admin/common'
import { BotTag } from '../BotTag'
import { ImagePickerRow } from '../customize/parts'
import { Avatar } from '../Presence'
import { SettingsPage, SettingsSection } from '../SettingsLayer'
import { SwitchRow } from '../Switch'
import { SecretBox } from '../WebhooksTab'
import { Plus } from 'lucide-react'

/** Obergrenze des Servers (`MAX_BOTS_PER_USER`). */
const MAX_BOTS = 10

/** Einstellungen → „Entwickler“: eigene Bot-Konten anlegen, Token erzeugen, bearbeiten, löschen. */
export function DeveloperSettings() {
  const sourceUrl = useStore((s) => s.instance?.source_url)
  const [bots, setBots] = useState<Bot[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [secret, setSecret] = useState<BotWithToken | null>(null)
  const [editing, setEditing] = useState<string | null>(null)

  const load = useCallback(() => {
    endpoints
      .bots()
      .then((list) => {
        setBots(list)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  useEffect(load, [load])

  function regenerate(b: Bot) {
    confirmAction({
      title: 'Token neu erzeugen',
      message: `Das bisherige Token von „${displayName(b)}“ gilt danach nicht mehr; laufende Verbindungen des Bots werden getrennt.`,
      confirmLabel: 'Neu erzeugen',
      action: async () => setSecret(await endpoints.regenerateBotToken(b.id)),
    })
  }

  function remove(b: Bot) {
    confirmAction({
      title: 'Bot löschen',
      message: `„${displayName(b)}“ wird gelöscht und verlässt alle Communities. Seine Nachrichten bleiben ohne Autor erhalten.`,
      confirmLabel: 'Löschen',
      action: async () => {
        await endpoints.deleteBot(b.id)
        setSecret((s) => (s?.id === b.id ? null : s))
        load()
      },
    })
  }

  const docs = sourceUrl ? `${sourceUrl.replace(/\/$/, '')}/blob/main/docs/bots.md` : null

  return (
    <SettingsPage
      title="Entwickler"
      description={
        <>
          Bots sind eigene Konten, die du mit einem Token steuerst – über dieselbe API wie die App (
          <code>Authorization: Bot &lt;token&gt;</code>).{' '}
          {docs ? (
            <a href={docs} target="_blank" rel="noopener noreferrer">
              Anleitung „Bot schreiben“
            </a>
          ) : (
            'Anleitung: docs/bots.md im Quellcode.'
          )}
        </>
      }
    >
      <CreateBot
        disabled={!!bots && bots.length >= MAX_BOTS}
        onCreated={(created) => {
          setSecret(created)
          load()
        }}
      />
      {secret && (
        <SecretBox
          title={`Token für „${displayName(secret)}“`}
          value={secret.token}
          hint="Wird nur jetzt angezeigt. Behandle es wie ein Passwort – wer es kennt, kann als dieser Bot handeln."
        />
      )}
      {error && <p className="error">{error}</p>}
      {!bots && !error && <p className="muted">Lade …</p>}
      {bots && (
        <SettingsSection title={`Deine Bots – ${bots.length} von ${MAX_BOTS}`}>
          {bots.length === 0 && <p className="muted small">Du hast noch keine Bots.</p>}
          <ul className="admin-rows">
            {bots.map((b) => (
              <li key={b.id} className="admin-row">
                <div className="admin-row-main">
                  <Avatar user={b} name={displayName(b)} size={32} />
                  <div className="admin-row-text">
                    <span className="truncate">
                      <strong>{displayName(b)}</strong> <span className="muted small">@{b.username}</span>
                      <BotTag />
                      {b.is_public && <span className="badge">Öffentlich</span>}
                    </span>
                    <span className="muted small truncate">ID: {b.id}</span>
                  </div>
                  <div className="admin-row-actions">
                    <CopyButton text={b.id} label="ID kopieren" />
                    <button
                      type="button"
                      className="button small-button"
                      aria-expanded={editing === b.id}
                      onClick={() => setEditing(editing === b.id ? null : b.id)}
                    >
                      Bearbeiten
                    </button>
                    <button type="button" className="button small-button" onClick={() => regenerate(b)}>
                      Token neu erzeugen
                    </button>
                    <button type="button" className="button danger small-button" onClick={() => remove(b)}>
                      Löschen
                    </button>
                  </div>
                </div>
                {editing === b.id && (
                  <BotEditor
                    key={b.id}
                    bot={b}
                    onSaved={() => {
                      setEditing(null)
                      load()
                    }}
                    onChanged={load}
                  />
                )}
              </li>
            ))}
          </ul>
        </SettingsSection>
      )}
    </SettingsPage>
  )
}

function CreateBot({ disabled, onCreated }: { disabled: boolean; onCreated: (bot: BotWithToken) => void }) {
  const [username, setUsername] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (!username.trim()) return
    setBusy(true)
    setError(null)
    try {
      onCreated(await endpoints.createBot({ username: username.trim() }))
      setUsername('')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <form className="integration-form" onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>Benutzername des neuen Bots</span>
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            maxLength={32}
            placeholder="z. B. wetterbot"
            disabled={disabled}
          />
        </label>
        <button type="submit" className="button primary" disabled={busy || disabled || !username.trim()}>
          <Plus size={16} aria-hidden="true" /> Bot anlegen
        </button>
      </form>
      {disabled && <p className="muted small">Du hast die Höchstzahl von {MAX_BOTS} Bots erreicht.</p>}
      {error && <p className="error small">{error}</p>}
    </>
  )
}

function BotEditor({ bot, onSaved, onChanged }: { bot: Bot; onSaved: () => void; onChanged: () => void }) {
  const [name, setName] = useState(bot.display_name ?? '')
  const [description, setDescription] = useState(bot.description ?? '')
  const [isPublic, setIsPublic] = useState(bot.is_public)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const dirty =
    name.trim() !== (bot.display_name ?? '') || description.trim() !== (bot.description ?? '') || isPublic !== bot.is_public

  async function save(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await endpoints.updateBot(bot.id, {
        display_name: name.trim() || null,
        description: description.trim() || null,
        is_public: isPublic,
      })
      onSaved()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  async function setAvatar(file: File | null) {
    setBusy(true)
    setError(null)
    try {
      if (file) {
        if (file.size > AVATAR_MAX_INPUT_BYTES) throw new Error('too_large')
        const prepared = await prepareAvatar(file)
        await uploadBotAvatar(bot.id, prepared.blob, prepared.filename)
      } else {
        await endpoints.deleteBotAvatar(bot.id)
      }
      onChanged()
    } catch (err) {
      setError(err instanceof Error && err.message === 'too_large' ? 'Die Datei ist zu groß.' : errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="integration-edit" onSubmit={(e) => void save(e)}>
      <label className="field">
        <span>
          Anzeigename <em className="muted">(optional)</em>
        </span>
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={64} placeholder={bot.username} />
      </label>
      <label className="field">
        <span>
          Beschreibung <em className="muted">(optional)</em>
        </span>
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} maxLength={400} rows={3} />
      </label>
      <SwitchRow
        title="Öffentlich"
        description="Andere können den Bot per ID zu Communities hinzufügen, in denen sie „Community verwalten“ dürfen."
        checked={isPublic}
        onChange={setIsPublic}
      />
      <SettingsSection title="Avatar">
        <ImagePickerRow
          label="Avatar"
          hint="PNG, JPEG, WebP oder GIF – wird auf 256 px verkleinert."
          hasImage={!!bot.avatar_url}
          busy={busy}
          onFile={(file) => void setAvatar(file)}
          onRemove={() => void setAvatar(null)}
          onTooLarge={() => setError('Die Datei ist zu groß.')}
        />
      </SettingsSection>
      {error && <p className="error small">{error}</p>}
      <div className="form-actions form-actions-start">
        <button type="submit" className="button primary small-button" disabled={busy || !dirty}>
          Speichern
        </button>
      </div>
    </form>
  )
}
