import { useCallback, useEffect, useState } from 'react'
import { endpoints } from '../lib/api'
import type { Channel, ChannelOverride, CommunityDetail, Role } from '../lib/api'
import { BITRATE_MAX, BITRATE_MIN, USER_LIMIT_MAX, bitrateLabel } from '../lib/audioLogic'
import type { ChannelTab } from '../lib/dialogs'
import { confirmAction } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { PERMISSIONS, covers, roleColor } from '../lib/permissionLabels'
import { Permission, has } from '../lib/permissions'
import { sortRoles } from '../lib/roles'
import { useUnsavedChanges, useUnsavedGuard } from '../lib/unsaved'
import { channelLabel, useStore } from '../store'
import { EmojiInput, EmojiTextarea } from './EmojiButton'
import { SettingsLayer, SettingsPage, SettingsSection } from './SettingsLayer'
import type { SettingsGroup } from './SettingsLayer'
import { SwitchRow } from './Switch'
import { Check, Circle, Minus, Trash2, X } from 'lucide-react'
import type { ReactNode } from 'react'

type Section = ChannelTab | 'delete'

/** Kanal-Einstellungen als Vollbild-Ebene: Übersicht (MANAGE_CHANNELS) und Berechtigungen (MANAGE_ROLES). */
export function ChannelSettings({
  communityId,
  channelId,
  initialTab = 'overview',
  onClose,
}: {
  communityId: string
  channelId: string
  initialTab?: ChannelTab
  onClose: () => void
}) {
  const detail = useStore((s) => s.details[communityId])
  const channel = detail?.channels.find((c) => c.id === channelId)
  const [tab, setTab] = useState<ChannelTab>(initialTab)

  // Kanal gelöscht oder nicht mehr sichtbar → schließen.
  useEffect(() => {
    if (!channel) onClose()
  }, [channel, onClose])
  if (!detail || !channel) return null

  const perms = detail.channel_permissions[channel.id] ?? detail.permissions
  const canManage = has(perms, Permission.MANAGE_CHANNELS)
  const canRoles = has(perms, Permission.MANAGE_ROLES)
  const current: ChannelTab = tab === 'permissions' && canRoles ? 'permissions' : canManage ? 'overview' : 'permissions'

  const groups: SettingsGroup<Section>[] = [
    {
      title: `${channelLabel(channel)} · ${channel.kind === 'text' ? 'Textkanal' : 'Sprachkanal'}`,
      items: [
        { id: 'overview', label: 'Übersicht', hidden: !canManage },
        { id: 'permissions', label: 'Berechtigungen', hidden: !canRoles },
      ],
    },
    {
      items: [{ id: 'delete', label: 'Kanal löschen', danger: true, hidden: !canManage, icon: <Trash2 size={16} /> }],
    },
  ]

  function onAction(id: Section): boolean {
    if (id !== 'delete' || !channel) return false
    confirmAction({
      title: 'Kanal löschen',
      message:
        channel.kind === 'text'
          ? `Willst du ${channelLabel(channel)} wirklich löschen? Alle Nachrichten und Anhänge darin werden endgültig gelöscht.`
          : `Willst du den Sprachkanal „${channel.name}“ wirklich löschen?`,
      confirmLabel: 'Kanal löschen',
      action: async () => {
        await endpoints.deleteChannel(channel.id)
        onClose()
      },
    })
    return true
  }

  return (
    <SettingsLayer<Section>
      label={`Kanal-Einstellungen ${channelLabel(channel)}`}
      groups={groups}
      active={current}
      onSelect={(id) => id !== 'delete' && setTab(id)}
      onAction={onAction}
      onClose={onClose}
    >
      {current === 'overview' && canManage && <OverviewTab key={channel.id} channel={channel} />}
      {current === 'permissions' && canRoles && <PermissionsTab detail={detail} channel={channel} myPerms={perms} />}
      {!canManage && !canRoles && <p className="muted">Du darfst diesen Kanal nicht verwalten.</p>}
    </SettingsLayer>
  )
}

/** Ton-Einstellungen eines Sprachkanals im Formular. `userLimit` 0 = unbegrenzt. */
interface AudioForm {
  bitrate: number
  stereo: boolean
  dtx: boolean
  red: boolean
  userLimit: number
}

function audioForm(c: Channel): AudioForm {
  return {
    bitrate: c.audio_bitrate,
    stereo: c.audio_stereo,
    dtx: c.audio_dtx,
    red: c.audio_red,
    userLimit: c.user_limit ?? 0,
  }
}

function sameAudio(a: AudioForm, b: AudioForm): boolean {
  return (
    a.bitrate === b.bitrate &&
    a.stereo === b.stereo &&
    a.dtx === b.dtx &&
    a.red === b.red &&
    a.userLimit === b.userLimit
  )
}

function OverviewTab({ channel }: { channel: Channel }) {
  const [name, setName] = useState(channel.name)
  const [topic, setTopic] = useState(channel.topic ?? '')
  const [audio, setAudio] = useState<AudioForm>(() => audioForm(channel))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isVoice = channel.kind === 'voice'
  const audioDirty = isVoice && !sameAudio(audio, audioForm(channel))
  const dirty = name.trim() !== channel.name || topic.trim() !== (channel.topic ?? '') || audioDirty

  async function save() {
    if (!name.trim()) {
      setError('Der Name darf nicht leer sein.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const updated = await endpoints.updateChannel(channel.id, {
        name: name.trim(),
        topic: topic.trim() || null,
        ...(audioDirty
          ? {
              audio_bitrate: audio.bitrate,
              audio_stereo: audio.stereo,
              audio_dtx: audio.dtx,
              audio_red: audio.red,
              user_limit: audio.userLimit > 0 ? audio.userLimit : null,
            }
          : {}),
      })
      setName(updated.name)
      setTopic(updated.topic ?? '')
      setAudio(audioForm(updated))
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setName(channel.name)
    setTopic(channel.topic ?? '')
    setAudio(audioForm(channel))
    setError(null)
  }

  const patchAudio = (patch: Partial<AudioForm>) => setAudio((a) => ({ ...a, ...patch }))

  useUnsavedChanges({ dirty, busy, save, reset })

  return (
    <SettingsPage title="Übersicht">
      <div className="form">
        <label className="field">
          <span>Kanalname</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={64} required />
        </label>
        {channel.kind === 'text' && (
          <p className="settings-description">Textkanäle werden kleingeschrieben, Leerzeichen werden zu Bindestrichen.</p>
        )}
        <label className="field">
          <span>
            Thema <em className="muted">(optional)</em>
          </span>
          <EmojiTextarea
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
            maxLength={1024}
            rows={3}
            placeholder="Worum geht es in diesem Kanal?"
          />
        </label>
        {isVoice && (
          <SettingsSection
            title="Ton"
            description="Gilt für alle, die dem Kanal danach beitreten. Wer schon drin ist, bekommt die Änderung beim nächsten Beitritt."
          >
            <label className="field">
              <span>
                Audio-Bitrate <span className="muted small">({bitrateLabel(audio.bitrate)})</span>
              </span>
              <input
                className="voice-range"
                type="range"
                min={BITRATE_MIN}
                max={BITRATE_MAX}
                step={16}
                value={audio.bitrate}
                onChange={(e) => patchAudio({ bitrate: Number(e.target.value) })}
                aria-label="Audio-Bitrate in kbit/s"
              />
              <p className="muted small">
                64 kbit/s reichen für Sprache. Höhere Werte klingen besser (z. B. für Musik), brauchen aber mehr
                Bandbreite.
              </p>
            </label>
            <SwitchRow
              title="Stereo"
              description="Überträgt Mikrofone in Stereo – sinnvoll für Musik, braucht etwa doppelt so viel Bandbreite."
              checked={audio.stereo}
              onChange={(stereo) => patchAudio({ stereo })}
            />
            <SwitchRow
              title="Sprechpausen sparen (DTX)"
              description="In Sprechpausen werden kaum Daten gesendet. Für Musik mit leisen Stellen besser ausschalten."
              checked={audio.dtx}
              onChange={(dtx) => patchAudio({ dtx })}
            />
            <SwitchRow
              title="Redundanz bei Paketverlust (RED)"
              description="Sendet Audio doppelt, damit es bei wackeliger Verbindung weniger stockt. Etwas mehr Bandbreite."
              checked={audio.red}
              onChange={(red) => patchAudio({ red })}
            />
            <label className="field">
              <span>
                Nutzerlimit{' '}
                <span className="muted small">
                  ({audio.userLimit > 0 ? `${audio.userLimit} Personen` : 'kein Limit'})
                </span>
              </span>
              <input
                className="voice-range"
                type="range"
                min={0}
                max={USER_LIMIT_MAX}
                step={1}
                value={audio.userLimit}
                onChange={(e) => patchAudio({ userLimit: Number(e.target.value) })}
                aria-label="Nutzerlimit (0 = kein Limit)"
              />
              <p className="muted small">
                Ist der Kanal voll, kommen nur noch Mitglieder mit dem Recht „Kanäle verwalten“ hinein.
              </p>
            </label>
          </SettingsSection>
        )}
        {error && <p className="error">{error}</p>}
      </div>
    </SettingsPage>
  )
}

function PermissionsTab({ detail, channel, myPerms }: { detail: CommunityDetail; channel: Channel; myPerms: number }) {
  const [overrides, setOverrides] = useState<ChannelOverride[] | null>(null)
  const [selectedId, setSelectedId] = useState<string | undefined>(detail.roles.find((r) => r.is_default)?.id)
  const [error, setError] = useState<string | null>(null)
  const blocked = useUnsavedGuard()

  const load = useCallback(() => {
    endpoints
      .channelOverrides(channel.id)
      .then(setOverrides)
      .catch((e) => setError(errorText(e)))
  }, [channel.id])

  useEffect(load, [load])

  const role = detail.roles.find((r) => r.id === selectedId) ?? detail.roles[0]
  const byRole = new Map((overrides ?? []).map((o) => [o.role_id, o]))

  return (
    <SettingsPage
      title="Berechtigungen"
      description="Sonderrechte pro Rolle in diesem Kanal. „Neutral“ übernimmt die Rechte der Rolle. Erlauben gewinnt gegen Verweigern aus anderen Rollen; „@alle“ wird zuerst angewendet."
    >
      {error && <p className="error">{error}</p>}
      {!overrides && !error && <p className="muted">Lade …</p>}
      {overrides && role && (
        <div className="admin-split">
          <div className="admin-list">
            <h4 className="settings-nav-title">Rollen</h4>
            {sortRoles(detail.roles).map((r) => (
              <button
                key={r.id}
                type="button"
                className={`admin-list-item${r.id === role.id ? ' active' : ''}`}
                onClick={() => r.id !== role.id && !blocked() && setSelectedId(r.id)}
              >
                <span className="role-dot" style={{ background: roleColor(r.color) }} aria-hidden="true" />
                <span className="truncate">{r.name}</span>
                {byRole.has(r.id) && (
                  <span className="override-mark" title="Hat Sonderrechte in diesem Kanal">
                    <Circle size={8} fill="currentColor" />
                  </span>
                )}
              </button>
            ))}
          </div>
          <OverrideEditor
            key={`${role.id}:${byRole.get(role.id)?.allow ?? 0}:${byRole.get(role.id)?.deny ?? 0}`}
            channel={channel}
            role={role}
            current={byRole.get(role.id)}
            myPerms={myPerms}
            onSaved={load}
          />
        </div>
      )}
    </SettingsPage>
  )
}

type TriValue = 'deny' | 'neutral' | 'allow'

function OverrideEditor({
  channel,
  role,
  current,
  myPerms,
  onSaved,
}: {
  channel: Channel
  role: Role
  current: ChannelOverride | undefined
  myPerms: number
  onSaved: () => void
}) {
  const [allow, setAllow] = useState(current?.allow ?? 0)
  const [deny, setDeny] = useState(current?.deny ?? 0)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const relevant = PERMISSIONS.filter((p) => p.channels?.includes(channel.kind))
  const dirty = allow !== (current?.allow ?? 0) || deny !== (current?.deny ?? 0)

  function valueOf(bit: number): TriValue {
    if ((allow & bit) !== 0) return 'allow'
    if ((deny & bit) !== 0) return 'deny'
    return 'neutral'
  }

  function set(bit: number, v: TriValue) {
    setAllow((a) => (v === 'allow' ? a | bit : a & ~bit))
    setDeny((d) => (v === 'deny' ? d | bit : d & ~bit))
    setError(null)
  }

  async function save() {
    setBusy(true)
    setError(null)
    try {
      if (allow === 0 && deny === 0) {
        if (current) await endpoints.deleteOverride(channel.id, role.id)
      } else {
        await endpoints.putOverride(channel.id, role.id, { allow, deny })
      }
      onSaved()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setAllow(current?.allow ?? 0)
    setDeny(current?.deny ?? 0)
    setError(null)
  }

  useUnsavedChanges({ dirty, busy, save, reset })

  return (
    <div className="form admin-editor">
      <div className="admin-heading">
        <strong className="admin-editor-title">
          <span className="role-dot" style={{ background: roleColor(role.color) }} aria-hidden="true" /> {role.name}
        </strong>
        <button
          type="button"
          className="button ghost small-button"
          disabled={busy || (allow === 0 && deny === 0)}
          onClick={() => {
            setAllow(0)
            setDeny(0)
          }}
        >
          Alles neutral
        </button>
      </div>
      <ul className="perm-list">
        {relevant.map((p) => {
          const own = covers(myPerms, p.bit)
          return (
            <li key={p.bit} className={`perm-tri-row${own ? '' : ' disabled'}`}>
              <span className="switch-row-text">
                <span className="switch-row-title">{p.label}</span>
                {p.description && <span className="switch-row-desc">{p.description}</span>}
              </span>
              <TriState value={valueOf(p.bit)} onChange={(v) => set(p.bit, v)} disabled={!own} label={p.label} />
            </li>
          )
        })}
      </ul>
      {error && <p className="error">{error}</p>}
    </div>
  )
}

const TRI: { value: TriValue; symbol: ReactNode; title: string }[] = [
  { value: 'deny', symbol: <X size={14} />, title: 'Verweigern' },
  { value: 'neutral', symbol: <Minus size={14} />, title: 'Neutral' },
  { value: 'allow', symbol: <Check size={14} />, title: 'Erlauben' },
]

function TriState({
  value,
  onChange,
  disabled,
  label,
}: {
  value: TriValue
  onChange: (v: TriValue) => void
  disabled: boolean
  label: string
}) {
  return (
    <div className="tri-state" role="radiogroup" aria-label={label}>
      {TRI.map((t) => (
        <button
          key={t.value}
          type="button"
          role="radio"
          aria-checked={value === t.value}
          title={t.title}
          className={`tri-${t.value}${value === t.value ? ' active' : ''}`}
          disabled={disabled}
          onClick={() => onChange(t.value)}
        >
          {t.symbol}
        </button>
      ))}
    </div>
  )
}
