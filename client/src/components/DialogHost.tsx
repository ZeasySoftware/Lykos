import { Suspense, useState } from 'react'
import type { FormEvent } from 'react'
import { endpoints } from '../lib/api'
import { useDialogs } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { useToasts } from '../lib/toast'
import { displayName, memberName, useMember, useStore } from '../store'
import { lazyComponent } from '../lib/lazy'
import { ConfirmDialog } from './ConfirmDialog'
import { CreateChannelDialog, InviteDialog } from './Dialogs'
import { GroupDmDialog, NewDmDialog } from './DirectMessages'
import { EmojiInput } from './EmojiButton'
import { LazyLayerFallback } from './LazyFallback'
import { Modal } from './Modal'
import { CircleAlert, X } from 'lucide-react'

/** Global geöffnete Dialoge (aus Kontextmenüs, Dropdowns, Tastenkürzeln) – als Stapel. */
// Einstellungs-Ebenen sind groß und selten offen → eigene Chunks (siehe lib/lazy).
const CommunitySettings = lazyComponent(() => import('./CommunitySettings').then((m) => m.CommunitySettings))
const ChannelSettings = lazyComponent(() => import('./ChannelSettings').then((m) => m.ChannelSettings))
const SettingsDialog = lazyComponent(() => import('./SettingsDialog').then((m) => m.SettingsDialog))

export function DialogHost() {
  const stack = useDialogs((s) => s.stack)
  const close = useDialogs((s) => s.close)
  return (
    <>
      {stack.map((d) => {
        const onClose = () => close(d.id)
        switch (d.kind) {
          case 'confirm':
            return <ConfirmDialog key={d.id} spec={d} onClose={onClose} />
          case 'nickname':
            return <NicknameDialog key={d.id} communityId={d.communityId} userId={d.userId} onClose={onClose} />
          case 'invite':
            return <InviteDialog key={d.id} communityId={d.communityId} onClose={onClose} />
          case 'createChannel':
            return (
              <CreateChannelDialog key={d.id} communityId={d.communityId} initialKind={d.channelKind} onClose={onClose} />
            )
          case 'communitySettings':
            return (
              <Suspense key={d.id} fallback={<LazyLayerFallback />}>
                <CommunitySettings communityId={d.communityId} initialTab={d.tab} onClose={onClose} />
              </Suspense>
            )
          case 'channelSettings':
            return (
              <Suspense key={d.id} fallback={<LazyLayerFallback />}>
                <ChannelSettings
                  communityId={d.communityId}
                  channelId={d.channelId}
                  initialTab={d.tab}
                  onClose={onClose}
                />
              </Suspense>
            )
          case 'settings':
            return (
              <Suspense key={d.id} fallback={<LazyLayerFallback />}>
                <SettingsDialog initialTab={d.tab} onClose={onClose} />
              </Suspense>
            )
          case 'groupDm':
            return <GroupDmHost key={d.id} dmId={d.dmId} onClose={onClose} />
          case 'statusText':
            return <StatusTextDialog key={d.id} onClose={onClose} />
          case 'newDm':
            return <NewDmDialog key={d.id} onClose={onClose} />
        }
      })}
    </>
  )
}

function GroupDmHost({ dmId, onClose }: { dmId: string; onClose: () => void }) {
  const dm = useStore((s) => s.dms[dmId])
  if (!dm) return null
  return <GroupDmDialog dm={dm} onClose={onClose} />
}

/** Nickname in einer Community setzen (eigener oder – mit Recht – fremder). */
function NicknameDialog({ communityId, userId, onClose }: { communityId: string; userId: string; onClose: () => void }) {
  const member = useMember(communityId, userId)
  const isSelf = useStore((s) => s.user?.id === userId)
  const upsertMember = useStore((s) => s.upsertMember)
  const [value, setValue] = useState(member?.nickname ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function save(nickname: string | null) {
    setBusy(true)
    setError(null)
    try {
      upsertMember(communityId, await endpoints.setNickname(communityId, isSelf ? '@me' : userId, nickname))
      onClose()
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault()
    void save(value.trim() || null)
  }

  const name = member ? memberName(member) : 'Mitglied'
  return (
    <Modal title={isSelf ? 'Eigenen Nickname ändern' : `Nickname von ${name}`} onClose={onClose}>
      <form className="form" onSubmit={submit}>
        <p className="muted small">
          Der Nickname gilt nur in dieser Community. Leer lassen, um {isSelf ? 'deinen' : 'den'} Anzeigenamen zu nutzen.
        </p>
        <label className="field">
          <span>Nickname</span>
          <EmojiInput
            value={value}
            onChange={(e) => setValue(e.target.value)}
            maxLength={32}
            placeholder={displayName(member?.user ?? null)}
            autoFocus
          />
        </label>
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          {member?.nickname && (
            <div className="form-actions-left">
              <button type="button" className="button ghost" disabled={busy} onClick={() => void save(null)}>
                Nickname zurücksetzen
              </button>
            </div>
          )}
          <button type="button" className="button ghost" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            Speichern
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Eigenen Status-Text setzen (Nutzerleiste → Kontextmenü). */
function StatusTextDialog({ onClose }: { onClose: () => void }) {
  const user = useStore((s) => s.user)
  const setUser = useStore((s) => s.setUser)
  const [value, setValue] = useState(user?.status_text ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function save(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      setUser(await endpoints.updateMe({ status_text: value.trim() || null }))
      onClose()
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <Modal title="Status-Text setzen" onClose={onClose}>
      <form className="form" onSubmit={save}>
        <label className="field">
          <span>Was machst du gerade?</span>
          <EmojiInput
            value={value}
            onChange={(e) => setValue(e.target.value)}
            maxLength={128}
            placeholder="z. B. „Im Urlaub bis Montag“"
            autoFocus
          />
        </label>
        <p className="muted small">Sichtbar für Mitglieder deiner Communities – unter deinem Namen und in deinem Profil.</p>
        {error && <p className="error">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button ghost" onClick={onClose}>
            Abbrechen
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            Speichern
          </button>
        </div>
      </form>
    </Modal>
  )
}

/** Kurze Rückmeldungen unten in der Mitte. */
export function Toasts() {
  const toasts = useToasts((s) => s.toasts)
  const dismiss = useToasts((s) => s.dismiss)
  if (toasts.length === 0) return null
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast${t.error ? ' toast-error' : ''}`}>
          {t.error && <CircleAlert size={16} aria-hidden="true" />}
          <span>{t.text}</span>
          <button type="button" className="icon-button small" onClick={() => dismiss(t.id)} aria-label="Schließen">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  )
}
