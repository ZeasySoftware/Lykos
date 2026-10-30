import { useCallback, useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints, uploadWebhookAvatar } from '../lib/api'
import type { Channel, CommunityDetail, Webhook, WebhookWithToken } from '../lib/api'
import { AVATAR_MAX_INPUT_BYTES, prepareAvatar } from '../lib/avatar'
import { confirmAction } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { Permission, has } from '../lib/permissions'
import { channelLabel, displayName } from '../store'
import { CopyButton } from './admin/common'
import { ImagePickerRow } from './customize/parts'
import { Avatar } from './Presence'
import { SettingsPage, SettingsSection } from './SettingsLayer'
import { Plus } from 'lucide-react'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })

/** Einmalig angezeigte Webhook-URL (enthält das Token) mit Kopieren-Knopf. */
export function SecretBox({ title, value, hint }: { title: string; value: string; hint: string }) {
  return (
    <div className="secret-box" role="status">
      <strong>{title}</strong>
      <div className="secret-box-value">
        <code>{value}</code>
        <CopyButton text={value} />
      </div>
      <span className="small">{hint}</span>
    </div>
  )
}

/**
 * Kanal-Einstellungen → „Integrationen“: eingehende Webhooks des Textkanals anlegen, umbenennen, Avatar setzen,
 * in einen anderen Kanal verschieben, URL neu erzeugen und löschen (`MANAGE_WEBHOOKS`).
 */
export function WebhooksTab({ detail, channel }: { detail: CommunityDetail; channel: Channel }) {
  const [webhooks, setWebhooks] = useState<Webhook[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [secret, setSecret] = useState<WebhookWithToken | null>(null)
  const [editing, setEditing] = useState<string | null>(null)

  const load = useCallback(() => {
    endpoints
      .channelWebhooks(channel.id)
      .then((list) => {
        setWebhooks(list)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [channel.id])

  useEffect(load, [load])

  async function create(e: FormEvent) {
    e.preventDefault()
    const trimmed = name.trim()
    if (!trimmed) return
    setBusy(true)
    setError(null)
    try {
      const created = await endpoints.createWebhook(channel.id, trimmed)
      setSecret(created)
      setName('')
      load()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function regenerate(w: Webhook) {
    confirmAction({
      title: 'URL neu erzeugen',
      message: `Die bisherige URL von „${w.name}“ funktioniert danach nicht mehr. Integrationen, die sie nutzen, musst du mit der neuen URL einrichten.`,
      confirmLabel: 'Neu erzeugen',
      action: async () => setSecret(await endpoints.regenerateWebhookToken(w.id)),
    })
  }

  function remove(w: Webhook) {
    confirmAction({
      title: 'Webhook löschen',
      message: `„${w.name}“ wird gelöscht und nimmt keine Nachrichten mehr an. Bisherige Nachrichten bleiben erhalten.`,
      confirmLabel: 'Löschen',
      action: async () => {
        await endpoints.deleteWebhook(w.id)
        setSecret((s) => (s?.id === w.id ? null : s))
        load()
      },
    })
  }

  return (
    <SettingsPage
      title="Integrationen"
      description={
        <>
          Webhooks lassen andere Dienste (z. B. Gitea, Grafana, Uptime Kuma) Nachrichten in {channelLabel(channel)} senden.
          Sie verstehen das Discord-Webhook-Format. Lykos lädt dabei keine fremden Bilder: Avatar-Links und Bilder in
          Karten werden ignoriert.
        </>
      }
    >
      <form className="integration-form" onSubmit={(e) => void create(e)}>
        <label className="field">
          <span>Name des neuen Webhooks</span>
          <input value={name} onChange={(e) => setName(e.target.value)} maxLength={80} placeholder="z. B. Gitea" />
        </label>
        <button type="submit" className="button primary" disabled={busy || !name.trim()}>
          <Plus size={16} aria-hidden="true" /> Webhook anlegen
        </button>
      </form>
      {secret && (
        <SecretBox
          title={`Webhook-URL für „${secret.name}“`}
          value={secret.url}
          hint="Wird nur jetzt angezeigt – kopiere sie in die Integration. Wer die URL kennt, kann in diesen Kanal schreiben."
        />
      )}
      {error && <p className="error">{error}</p>}
      {!webhooks && !error && <p className="muted">Lade …</p>}
      {webhooks && webhooks.length === 0 && <p className="muted">Noch keine Webhooks in diesem Kanal.</p>}
      <ul className="admin-rows">
        {webhooks?.map((w) => (
          <li key={w.id} className="admin-row">
            <div className="admin-row-main">
              <Avatar user={{ id: w.id, avatar_url: w.avatar_url }} name={w.name} size={32} />
              <div className="admin-row-text">
                <strong className="truncate">{w.name}</strong>
                <span className="muted small">
                  Angelegt {dateFmt.format(new Date(w.created_at))}
                  {w.created_by && ` von ${displayName(w.created_by)}`}
                </span>
              </div>
              <div className="admin-row-actions">
                <button
                  type="button"
                  className="button small-button"
                  aria-expanded={editing === w.id}
                  onClick={() => setEditing(editing === w.id ? null : w.id)}
                >
                  Bearbeiten
                </button>
                <button type="button" className="button small-button" onClick={() => regenerate(w)}>
                  URL neu erzeugen
                </button>
                <button type="button" className="button danger small-button" onClick={() => remove(w)}>
                  Löschen
                </button>
              </div>
            </div>
            {editing === w.id && (
              <WebhookEditor
                key={w.id}
                webhook={w}
                detail={detail}
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
    </SettingsPage>
  )
}

function WebhookEditor({
  webhook,
  detail,
  onSaved,
  onChanged,
}: {
  webhook: Webhook
  detail: CommunityDetail
  onSaved: () => void
  onChanged: () => void
}) {
  const [name, setName] = useState(webhook.name)
  const [channelId, setChannelId] = useState(webhook.channel_id)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Verschieben nur in Textkanäle, in denen man ebenfalls Webhooks verwalten darf.
  const targets = detail.channels.filter(
    (c) =>
      c.kind === 'text' && has(detail.channel_permissions[c.id] ?? detail.permissions, Permission.MANAGE_WEBHOOKS),
  )
  const dirty = name.trim() !== webhook.name || channelId !== webhook.channel_id

  async function save(e: FormEvent) {
    e.preventDefault()
    if (!name.trim()) return
    setBusy(true)
    setError(null)
    try {
      await endpoints.updateWebhook(webhook.id, {
        name: name.trim() !== webhook.name ? name.trim() : undefined,
        channel_id: channelId !== webhook.channel_id ? channelId : undefined,
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
        await uploadWebhookAvatar(webhook.id, prepared.blob, prepared.filename)
      } else {
        await endpoints.deleteWebhookAvatar(webhook.id)
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
        <span>Name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={80} required />
      </label>
      <label className="field">
        <span>Kanal</span>
        <select value={channelId} onChange={(e) => setChannelId(e.target.value)}>
          {targets.map((c) => (
            <option key={c.id} value={c.id}>
              {channelLabel(c)}
            </option>
          ))}
        </select>
      </label>
      <SettingsSection title="Avatar" description="Wird statt eines Avatar-Links aus der Integration angezeigt.">
        <ImagePickerRow
          label="Avatar"
          hint="PNG, JPEG, WebP oder GIF – wird auf 256 px verkleinert."
          hasImage={!!webhook.avatar_url}
          busy={busy}
          onFile={(file) => void setAvatar(file)}
          onRemove={() => void setAvatar(null)}
          onTooLarge={() => setError('Die Datei ist zu groß.')}
        />
      </SettingsSection>
      {error && <p className="error small">{error}</p>}
      <div className="form-actions form-actions-start">
        <button type="submit" className="button primary small-button" disabled={busy || !dirty || !name.trim()}>
          Speichern
        </button>
      </div>
    </form>
  )
}
