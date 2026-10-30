import { useCallback, useEffect, useMemo, useState } from 'react'
import type { DragEvent, FormEvent } from 'react'
import { endpoints } from '../lib/api'
import type { CommunityDetail, Invite, Member, Role, StorageUsage } from '../lib/api'
import { leaveCommunity } from '../lib/contextMenus'
import type { CommunityTab } from '../lib/dialogs'
import { confirmAction, openDialog } from '../lib/dialogs'
import { errorText } from '../lib/errors'
import { communityTabs } from '../lib/menuRules'
import { PERMISSIONS, PERMISSION_GROUPS, ROLE_COLOR_SUGGESTIONS, covers, roleColor } from '../lib/permissionLabels'
import { Permission, has } from '../lib/permissions'
import { outranks, sortRoles, topPosition } from '../lib/roles'
import { runAction } from '../lib/toast'
import { useUnsavedChanges, useUnsavedGuard } from '../lib/unsaved'
import { displayName, memberName, useStore } from '../store'
import { AuditTab } from './AuditLog'
import { EmojiInput } from './EmojiButton'
import { EmojiManager } from './EmojiManager'
import { CommunityAppearance } from './customize/CommunityAppearance'
import { Avatar } from './Presence'
import { SettingsLayer, SettingsPage, SettingsSection } from './SettingsLayer'
import type { SettingsGroup } from './SettingsLayer'
import { UsageBar } from './StorageUsage'
import { SwitchRow } from './Switch'
import { Ban, Check, ChevronDown, ChevronUp, Crown, GripVertical, LogOut, Plus, Search, Trash2, UserPlus, X } from 'lucide-react'

type Section = CommunityTab | 'leave'

const dateFmt = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' })

type Feedback = { ok: boolean; text: string } | null

/** Community-Einstellungen als Vollbild-Ebene: zeigt nur die Bereiche, für die der Nutzer Rechte hat. */
export function CommunitySettings({
  communityId,
  initialTab = 'overview',
  onClose,
}: {
  communityId: string
  initialTab?: CommunityTab
  onClose: () => void
}) {
  const detail = useStore((s) => s.details[communityId])
  const user = useStore((s) => s.user)
  const [tab, setTab] = useState<CommunityTab>(initialTab)

  // Community gelöscht oder verlassen → schließen.
  useEffect(() => {
    if (!detail) onClose()
  }, [detail, onClose])
  if (!detail) return null

  const isOwner = detail.community.owner_id === user?.id
  const visible = communityTabs(detail.permissions, isOwner)
  // Rechte können sich zur Laufzeit ändern – dann auf die Übersicht zurückfallen.
  const current: CommunityTab = visible.some((t) => t.id === tab) && (tab !== 'danger' || isOwner) ? tab : 'overview'

  const groups: SettingsGroup<Section>[] = [
    {
      title: detail.community.name,
      items: visible.filter((t) => t.id !== 'danger').map((t) => ({ id: t.id, label: t.label })),
    },
    {
      items: isOwner
        ? [{ id: 'danger', label: 'Community löschen', danger: true, icon: <Trash2 size={16} /> }]
        : [{ id: 'leave', label: 'Community verlassen', danger: true, icon: <LogOut size={16} /> }],
    },
  ]

  function onAction(id: Section): boolean {
    if (id !== 'leave') return false
    leaveCommunity(communityId)
    return true
  }

  return (
    <SettingsLayer<Section>
      label={`Einstellungen von ${detail.community.name}`}
      groups={groups}
      active={current}
      onSelect={(id) => id !== 'leave' && setTab(id)}
      onAction={onAction}
      onClose={onClose}
    >
      {current === 'overview' && <OverviewPage key={detail.community.id} detail={detail} />}
      {current === 'appearance' && (
        <SettingsPage title="Darstellung" description="Icon, Banner, Beschreibung und Akzentfarbe – rechts die Vorschau.">
          <CommunityAppearance detail={detail} />
        </SettingsPage>
      )}
      {current === 'roles' && <RolesPage detail={detail} />}
      {current === 'members' && <MembersPage detail={detail} />}
      {current === 'invites' && <InvitesPage detail={detail} />}
      {current === 'emojis' && (
        <SettingsPage
          title="Emojis"
          description="Eigene Emotes dieser Community – nutzbar in Nachrichten und als Reaktion."
        >
          <EmojiManager communityId={detail.community.id} canManage />
        </SettingsPage>
      )}
      {current === 'audit' && <AuditTab detail={detail} />}
      {current === 'danger' && isOwner && <DangerPage detail={detail} onDone={onClose} />}
    </SettingsLayer>
  )
}

// ---------------------------------------------------------------------------
// Übersicht
// ---------------------------------------------------------------------------

function OverviewPage({ detail }: { detail: CommunityDetail }) {
  const updateCommunity = useStore((s) => s.updateCommunity)
  const { community } = detail
  const canManage = has(detail.permissions, Permission.MANAGE_COMMUNITY)

  const initial = {
    name: community.name,
    unlimited: community.message_retention_days == null,
    days: community.message_retention_days ?? 90,
    noQuota: community.upload_quota_mb == null,
    quotaGb: community.upload_quota_mb != null ? community.upload_quota_mb / 1024 : 10,
  }
  const [name, setName] = useState(initial.name)
  const [unlimited, setUnlimited] = useState(initial.unlimited)
  const [days, setDays] = useState(initial.days)
  const [noQuota, setNoQuota] = useState(initial.noQuota)
  const [quotaGb, setQuotaGb] = useState(initial.quotaGb)
  const [usage, setUsage] = useState<StorageUsage | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Belegung nur für Verwalter (der Server verlangt MANAGE_COMMUNITY); nach dem Speichern neu laden.
  useEffect(() => {
    if (!canManage) return
    endpoints
      .communityUsage(community.id)
      .then(setUsage)
      .catch(() => setUsage(null))
  }, [canManage, community.id, community.upload_quota_mb])

  const dirty =
    canManage &&
    (name.trim() !== initial.name ||
      unlimited !== initial.unlimited ||
      (!unlimited && days !== initial.days) ||
      noQuota !== initial.noQuota ||
      (!noQuota && quotaGb !== initial.quotaGb))

  async function save() {
    if (!name.trim()) {
      setError('Der Name darf nicht leer sein.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const updated = await endpoints.updateCommunity(community.id, {
        name: name.trim(),
        message_retention_days: unlimited ? null : Math.min(3650, Math.max(1, Math.floor(days))),
        upload_quota_mb: noQuota ? null : Math.min(10_000_000, Math.max(1, Math.round(quotaGb * 1024))),
      })
      updateCommunity(updated)
      setName(updated.name)
      setDays(updated.message_retention_days ?? 90)
      setQuotaGb(updated.upload_quota_mb != null ? updated.upload_quota_mb / 1024 : 10)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setName(initial.name)
    setUnlimited(initial.unlimited)
    setDays(initial.days)
    setNoQuota(initial.noQuota)
    setQuotaGb(initial.quotaGb)
    setError(null)
  }

  useUnsavedChanges({ dirty, busy, save, reset })

  const retention =
    community.message_retention_days == null
      ? 'Nachrichten werden unbegrenzt aufbewahrt.'
      : `Nachrichten werden nach ${community.message_retention_days} Tagen gelöscht.`

  if (!canManage) {
    return (
      <SettingsPage title="Übersicht">
        <SettingsSection title="Community">
          <p className="settings-value">{community.name}</p>
        </SettingsSection>
        <SettingsSection title="Aufbewahrung">
          <p className="muted">{retention}</p>
        </SettingsSection>
        <p className="muted small">Angelegt am {dateFmt.format(new Date(community.created_at))}.</p>
      </SettingsPage>
    )
  }

  return (
    <SettingsPage title="Übersicht">
      <SettingsSection title="Name der Community">
        <label className="field">
          <span className="sr-only">Name</span>
          <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
        </label>
      </SettingsSection>
      <SettingsSection
        title="Nachrichten aufbewahren"
        description="Ältere Nachrichten und ihre Anhänge werden automatisch gelöscht. Gut für Datensparsamkeit."
      >
        <SwitchRow title="Unbegrenzt aufbewahren" checked={unlimited} onChange={setUnlimited} />
        {!unlimited && (
          <label className="inline-field">
            <input type="number" min={1} max={3650} value={days} onChange={(e) => setDays(Number(e.target.value))} />
            Tage
          </label>
        )}
      </SettingsSection>
      <SettingsSection
        title="Speicher für Anhänge"
        description="Ohne Community-Kontingent gelten nur die Kontingente der einzelnen Mitglieder."
      >
        <SwitchRow title="Kein Community-Kontingent" checked={noQuota} onChange={setNoQuota} />
        {!noQuota && (
          <label className="inline-field">
            <input type="number" min={0.001} step={0.5} value={quotaGb} onChange={(e) => setQuotaGb(Number(e.target.value))} />
            GB für alle Anhänge der Community zusammen
          </label>
        )}
        {usage && <UsageBar usage={usage} />}
      </SettingsSection>
      {error && <p className="error">{error}</p>}
      <p className="muted small">Angelegt am {dateFmt.format(new Date(community.created_at))}.</p>
    </SettingsPage>
  )
}

// ---------------------------------------------------------------------------
// Rollen: Liste links, Editor rechts (Anzeige / Berechtigungen / Mitglieder)
// ---------------------------------------------------------------------------

function RolesPage({ detail }: { detail: CommunityDetail }) {
  const loadDetail = useStore((s) => s.loadDetail)
  const setRoles = useStore((s) => s.setRoles)
  const user = useStore((s) => s.user)
  const members = useStore((s) => s.members[detail.community.id])
  const loadMembers = useStore((s) => s.loadMembers)
  const blocked = useUnsavedGuard()
  const communityId = detail.community.id
  const isOwner = detail.community.owner_id === user?.id
  const roles = useMemo(() => sortRoles(detail.roles), [detail.roles])
  const defaultRole = roles.find((r) => r.is_default)
  const [selectedId, setSelectedId] = useState<string | undefined>(roles.find((r) => !r.is_default)?.id ?? defaultRole?.id)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  /** Reihenfolge während des Speicherns (optimistisch), sonst `null`. */
  const [pendingOrder, setPendingOrder] = useState<string[] | null>(null)
  const [dragId, setDragId] = useState<string | null>(null)
  const [dropIndex, setDropIndex] = useState<number | null>(null)

  // Hierarchie und Mitgliederzahlen brauchen die Mitgliederliste.
  useEffect(() => {
    loadMembers(communityId).catch(() => {})
  }, [communityId, loadMembers])
  const myRoleIds = members?.find((m) => m.user.id === user?.id)?.role_ids
  const top = topPosition(roles, myRoleIds)
  const canManage = (r: Role) => outranks(r, isOwner, top)

  const ordered = useMemo(() => {
    const others = roles.filter((r) => !r.is_default)
    if (!pendingOrder) return others
    const byId = new Map(others.map((r) => [r.id, r]))
    return pendingOrder.map((id) => byId.get(id)).filter((r): r is Role => !!r)
  }, [roles, pendingOrder])
  // Rollen bis einschließlich der eigenen höchsten bleiben an ihrem Platz.
  const firstMovable = ordered.findIndex((r) => outranks(r, isOwner, top))
  const fixed = firstMovable < 0 ? ordered.length : firstMovable

  const selected = roles.find((r) => r.id === selectedId) ?? defaultRole
  const counts = useMemo(() => {
    const map = new Map<string, number>()
    for (const m of members ?? []) for (const id of m.role_ids) map.set(id, (map.get(id) ?? 0) + 1)
    return map
  }, [members])

  function select(id: string) {
    if (id === selected?.id || blocked()) return
    setSelectedId(id)
  }

  async function create() {
    if (blocked()) return
    setError(null)
    setBusy(true)
    try {
      const role = await endpoints.createRole(communityId, { name: 'Neue Rolle' })
      await loadDetail(communityId)
      setSelectedId(role.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  async function move(from: number, to: number) {
    if (from === to || from < fixed || to < fixed || to >= ordered.length || busy) return
    const ids = ordered.map((r) => r.id)
    const [id] = ids.splice(from, 1)
    ids.splice(to, 0, id)
    setError(null)
    setBusy(true)
    setPendingOrder(ids)
    try {
      setRoles(communityId, await endpoints.reorderRoles(communityId, ids))
    } catch (err) {
      setError(errorText(err))
    } finally {
      setPendingOrder(null)
      setBusy(false)
    }
  }

  /** Einfügestelle beim Ziehen: obere Hälfte einer Zeile = davor, untere Hälfte = dahinter. */
  function dropSlot(e: DragEvent<HTMLElement>, index: number): number {
    const rect = e.currentTarget.getBoundingClientRect()
    return e.clientY < rect.top + rect.height / 2 ? index : index + 1
  }

  function endDrag() {
    setDragId(null)
    setDropIndex(null)
  }

  function onDrop(e: DragEvent<HTMLElement>, index: number) {
    e.preventDefault()
    const from = ordered.findIndex((r) => r.id === dragId)
    const slot = dropSlot(e, index)
    endDrag()
    if (from >= 0) void move(from, slot > from ? slot - 1 : slot)
  }

  function roleButton(r: Role) {
    const count = r.is_default ? (members?.length ?? 0) : (counts.get(r.id) ?? 0)
    return (
      <button
        type="button"
        className={`admin-list-item${r.id === selected?.id ? ' active' : ''}`}
        onClick={() => select(r.id)}
        aria-current={r.id === selected?.id ? 'true' : undefined}
      >
        <span className="role-dot" style={{ background: roleColor(r.color) }} aria-hidden="true" />
        <span className="truncate">{r.name}</span>
        <span className="admin-list-count" title={`${count} Mitglieder`}>
          {count}
        </span>
      </button>
    )
  }

  return (
    <SettingsPage
      title="Rollen"
      description="Mitglieder haben die Rechte aller ihrer Rollen zusammen. Oben steht die höchste Rolle – ziehen oder Pfeile nutzen, um die Reihenfolge zu ändern. Du verwaltest nur Rollen unter deiner höchsten."
    >
      {error && <p className="error">{error}</p>}
      <div className="admin-split roles-split">
        <div className="roles-column">
          <div className="roles-column-head">
            <h4 className="settings-nav-title">Rollen – {roles.length}</h4>
            <button type="button" className="icon-button small" onClick={create} disabled={busy} title="Rolle erstellen" aria-label="Rolle erstellen">
              <Plus size={16} />
            </button>
          </div>
          <ul className="admin-list role-order" aria-label="Rollen, höchste zuerst">
            {ordered.map((r, i) => {
              const movable = i >= fixed
              const marker =
                dragId && dropIndex === i
                  ? ' drop-before'
                  : dragId && dropIndex === i + 1 && i === ordered.length - 1
                    ? ' drop-after'
                    : ''
              return (
                <li
                  key={r.id}
                  className={`role-order-row${dragId === r.id ? ' dragging' : ''}${marker}`}
                  draggable={movable && !busy}
                  onDragStart={(e) => {
                    e.dataTransfer.effectAllowed = 'move'
                    e.dataTransfer.setData('text/plain', r.id)
                    setDragId(r.id)
                  }}
                  onDragEnd={endDrag}
                  onDragOver={(e) => {
                    if (!dragId) return
                    const slot = dropSlot(e, i)
                    if (slot < fixed) return
                    e.preventDefault()
                    e.dataTransfer.dropEffect = 'move'
                    setDropIndex(slot)
                  }}
                  onDrop={(e) => onDrop(e, i)}
                >
                  <span className={`role-grip${movable ? '' : ' placeholder'}`} aria-hidden="true">
                    <GripVertical size={14} />
                  </span>
                  {roleButton(r)}
                  {movable && (
                    <span className="role-move">
                      <button
                        type="button"
                        className="icon-button small"
                        onClick={() => void move(i, i - 1)}
                        disabled={busy || i <= fixed}
                        aria-label={`${r.name} nach oben`}
                        title="Nach oben"
                      >
                        <ChevronUp size={14} />
                      </button>
                      <button
                        type="button"
                        className="icon-button small"
                        onClick={() => void move(i, i + 1)}
                        disabled={busy || i >= ordered.length - 1}
                        aria-label={`${r.name} nach unten`}
                        title="Nach unten"
                      >
                        <ChevronDown size={14} />
                      </button>
                    </span>
                  )}
                </li>
              )
            })}
            {defaultRole && (
              <li className="role-order-row">
                <span className="role-grip placeholder" aria-hidden="true">
                  <GripVertical size={14} />
                </span>
                {roleButton(defaultRole)}
              </li>
            )}
          </ul>
        </div>
        {selected && (
          <RoleEditor
            key={selected.id}
            role={selected}
            detail={detail}
            manageable={canManage(selected)}
            onDeleted={() => setSelectedId(defaultRole?.id)}
          />
        )}
      </div>
    </SettingsPage>
  )
}

type RoleTab = 'display' | 'permissions' | 'members'

function RoleEditor({
  role,
  detail,
  manageable,
  onDeleted,
}: {
  role: Role
  detail: CommunityDetail
  /** Liegt die Rolle unterhalb der eigenen höchsten (Hierarchie)? */
  manageable: boolean
  onDeleted: () => void
}) {
  const loadDetail = useStore((s) => s.loadDetail)
  const members = useStore((s) => s.members[detail.community.id])
  const blocked = useUnsavedGuard()
  const myPermissions = detail.permissions
  const [tab, setTab] = useState<RoleTab>('display')
  const [name, setName] = useState(role.name)
  const [color, setColor] = useState<string | null>(roleColor(role.color) ?? null)
  const [perms, setPerms] = useState(role.permissions)
  const [query, setQuery] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Rollen mit Rechten, die man selbst nicht hat, darf man nicht bearbeiten (Server prüft ebenso).
  // Ebenso Rollen, die nicht unterhalb der eigenen höchsten Rolle stehen.
  const editable = covers(myPermissions, role.permissions) && manageable
  const initialColor = roleColor(role.color) ?? null
  const dirty =
    editable && (perms !== role.permissions || (!role.is_default && (name.trim() !== role.name || color !== initialColor)))

  async function save() {
    if (!role.is_default && !name.trim()) {
      setError('Der Name darf nicht leer sein.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      await endpoints.updateRole(
        role.id,
        role.is_default
          ? { permissions: perms }
          : { name: name.trim(), color: color ? parseInt(color.slice(1), 16) : null, permissions: perms },
      )
      await loadDetail(role.community_id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function reset() {
    setName(role.name)
    setColor(initialColor)
    setPerms(role.permissions)
    setError(null)
  }

  useUnsavedChanges({ dirty, busy, save, reset })

  function remove() {
    if (blocked()) return
    confirmAction({
      title: 'Rolle löschen',
      message: `Willst du die Rolle „${role.name}“ wirklich löschen? Mitglieder verlieren damit die Rechte dieser Rolle.`,
      confirmLabel: 'Rolle löschen',
      action: async () => {
        await endpoints.deleteRole(role.id)
        await loadDetail(role.community_id)
        onDeleted()
      },
    })
  }

  const holders = (members ?? []).filter((m) => m.role_ids.includes(role.id))
  const tabs: { id: RoleTab; label: string }[] = [
    { id: 'display', label: 'Anzeige' },
    { id: 'permissions', label: 'Berechtigungen' },
    { id: 'members', label: role.is_default ? 'Mitglieder' : `Mitglieder verwalten (${holders.length})` },
  ]

  return (
    <div className="role-editor">
      <div className="role-editor-head">
        <h3 className="role-editor-title">
          Rolle bearbeiten – <span style={{ color: color ?? undefined }}>{role.is_default ? '@alle' : name || role.name}</span>
        </h3>
        {!role.is_default && editable && (
          <button type="button" className="icon-button danger" onClick={remove} title="Rolle löschen" aria-label="Rolle löschen">
            <Trash2 size={18} />
          </button>
        )}
      </div>
      <div className="underline-tabs" role="tablist">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            className={tab === t.id ? 'active' : ''}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      {!manageable ? (
        <p className="notice">Diese Rolle steht nicht unter deiner höchsten Rolle – du kannst sie nicht bearbeiten.</p>
      ) : (
        !editable && <p className="notice">Diese Rolle hat Rechte, die du selbst nicht besitzt – du kannst sie nicht bearbeiten.</p>
      )}
      {error && <p className="error">{error}</p>}

      {tab === 'display' &&
        (role.is_default ? (
          <p className="settings-description">
            „@alle“ ist die Standardrolle: Jedes Mitglied hat sie. Name und Farbe lassen sich nicht ändern – ihre Rechte
            schon (Reiter „Berechtigungen“).
          </p>
        ) : (
          <div className="form">
            <label className="field">
              <span>Rollenname</span>
              <EmojiInput value={name} onChange={(e) => setName(e.target.value)} maxLength={64} required disabled={!editable} />
            </label>
            <div className="field">
              <span>Rollenfarbe</span>
              <p className="settings-description">Mitglieder erscheinen in der Farbe ihrer höchsten Rolle.</p>
              <div className="role-color-picker">
                <button
                  type="button"
                  className={`role-swatch big none${color === null ? ' active' : ''}`}
                  onClick={() => setColor(null)}
                  disabled={!editable}
                  title="Keine Farbe"
                  aria-label="Keine Farbe"
                  aria-pressed={color === null}
                >
                  {color === null ? <Check size={18} /> : <Ban size={18} />}
                </button>
                <label
                  className={`role-swatch big custom${color && !ROLE_COLOR_SUGGESTIONS.includes(color) ? ' active' : ''}`}
                  style={{ background: color && !ROLE_COLOR_SUGGESTIONS.includes(color) ? color : undefined }}
                  title="Eigene Farbe"
                >
                  <input
                    type="color"
                    value={color ?? '#8b7cf6'}
                    onChange={(e) => setColor(e.target.value)}
                    disabled={!editable}
                    aria-label="Eigene Farbe wählen"
                  />
                </label>
                <div className="role-color-grid">
                  {ROLE_COLOR_SUGGESTIONS.map((c) => (
                    <button
                      key={c}
                      type="button"
                      className={`role-swatch${color === c ? ' active' : ''}`}
                      style={{ background: c }}
                      onClick={() => setColor(c)}
                      disabled={!editable}
                      aria-label={`Farbe ${c}`}
                      aria-pressed={color === c}
                    >
                      {color === c && <Check size={14} />}
                    </button>
                  ))}
                </div>
              </div>
            </div>
            <div className="role-preview">
              <span className="muted small">Vorschau</span>
              <span className="role-preview-name" style={{ color: color ?? undefined }}>
                {name || 'Rolle'}
              </span>
            </div>
          </div>
        ))}

      {tab === 'permissions' && (
        <div className="form">
          <label className="search-input">
            <Search size={15} aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Berechtigungen suchen"
              aria-label="Berechtigungen suchen"
            />
          </label>
          <PermissionSwitches
            perms={perms}
            myPermissions={myPermissions}
            editable={editable}
            query={query}
            onToggle={(bit, on) => setPerms((p) => (on ? p | bit : p & ~bit))}
          />
          {!role.is_default && editable && perms !== 0 && (
            <div className="form-actions form-actions-start">
              <button type="button" className="button ghost small-button" onClick={() => setPerms(0)}>
                Alle Berechtigungen entfernen
              </button>
            </div>
          )}
        </div>
      )}

      {tab === 'members' && <RoleMembers role={role} detail={detail} editable={editable} />}
    </div>
  )
}

/** Gruppierte Schalter mit Beschreibungen; `query` filtert nach Name und Beschreibung. */
function PermissionSwitches({
  perms,
  myPermissions,
  editable,
  query,
  onToggle,
}: {
  perms: number
  myPermissions: number
  editable: boolean
  query: string
  onToggle: (bit: number, on: boolean) => void
}) {
  const q = query.trim().toLocaleLowerCase('de')
  const byBit = new Map(PERMISSIONS.map((p) => [p.bit, p]))
  const groups = PERMISSION_GROUPS.map((g) => ({
    title: g.title,
    items: g.bits
      .map((b) => byBit.get(b)!)
      .filter((p) => !q || p.label.toLocaleLowerCase('de').includes(q) || p.description.toLocaleLowerCase('de').includes(q)),
  })).filter((g) => g.items.length > 0)

  if (groups.length === 0) return <p className="muted">Keine Berechtigung passt zu „{query}“.</p>
  return (
    <>
      {groups.map((g) => (
        <section key={g.title} className="perm-group">
          <h4 className="settings-section-title">{g.title}</h4>
          {g.items.map((p) => {
            const own = covers(myPermissions, p.bit)
            return (
              <SwitchRow
                key={p.bit}
                title={p.label}
                description={p.description}
                checked={(perms & p.bit) !== 0}
                disabled={!editable || !own}
                disabledReason={!own ? 'Dieses Recht hast du selbst nicht – du kannst es nicht vergeben.' : undefined}
                onChange={(on) => onToggle(p.bit, on)}
              />
            )
          })}
        </section>
      ))}
    </>
  )
}

/** Mitglieder einer Rolle: Liste mit Entfernen, darunter Hinzufügen per Suche. */
function RoleMembers({ role, detail, editable }: { role: Role; detail: CommunityDetail; editable: boolean }) {
  const communityId = detail.community.id
  const members = useStore((s) => s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  const [query, setQuery] = useState('')

  if (role.is_default) {
    return <p className="settings-description">Jedes Mitglied hat „@alle“ automatisch – hier gibt es nichts zuzuweisen.</p>
  }

  const q = query.trim().toLocaleLowerCase('de')
  const matches = (m: Member) =>
    !q ||
    memberName(m).toLocaleLowerCase('de').includes(q) ||
    m.user.username.toLocaleLowerCase('de').includes(q)
  const holders = (members ?? []).filter((m) => m.role_ids.includes(role.id)).sort((a, b) => memberName(a).localeCompare(memberName(b), 'de'))
  const others = (members ?? []).filter((m) => !m.role_ids.includes(role.id) && matches(m)).slice(0, 8)

  function change(m: Member, on: boolean) {
    runAction(async () => {
      if (on) await endpoints.addMemberRole(communityId, m.user.id, role.id)
      else await endpoints.removeMemberRole(communityId, m.user.id, role.id)
      await loadMembers(communityId)
    })
  }

  return (
    <div className="form">
      <label className="search-input">
        <Search size={15} aria-hidden="true" />
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Mitglieder suchen"
          aria-label="Mitglieder suchen"
        />
      </label>
      {!members && <p className="muted">Lade …</p>}
      <ul className="role-member-list">
        {holders.filter(matches).map((m) => (
          <li key={m.user.id}>
            <Avatar user={m.user} name={memberName(m)} size={28} />
            <span className="truncate">
              <strong>{memberName(m)}</strong> <span className="muted small">@{m.user.username}</span>
            </span>
            {editable && (
              <button
                type="button"
                className="icon-button small"
                title="Rolle entziehen"
                aria-label={`${memberName(m)} die Rolle entziehen`}
                onClick={() => change(m, false)}
              >
                <X size={14} />
              </button>
            )}
          </li>
        ))}
        {members && holders.length === 0 && <li className="muted small">Noch niemand hat diese Rolle.</li>}
      </ul>
      {editable && q && (
        <>
          <h4 className="settings-section-title">Hinzufügen</h4>
          <ul className="role-member-list">
            {others.map((m) => (
              <li key={m.user.id}>
                <Avatar user={m.user} name={memberName(m)} size={28} />
                <span className="truncate">
                  <strong>{memberName(m)}</strong> <span className="muted small">@{m.user.username}</span>
                </span>
                <button type="button" className="button small-button" onClick={() => change(m, true)}>
                  <UserPlus size={14} aria-hidden="true" /> Hinzufügen
                </button>
              </li>
            ))}
            {others.length === 0 && <li className="muted small">Niemand gefunden.</li>}
          </ul>
        </>
      )}
      {editable && !q && <p className="muted small">Tipp: Suche nach einem Namen, um die Rolle weiteren Mitgliedern zu geben.</p>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Mitglieder
// ---------------------------------------------------------------------------

function MembersPage({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const members = useStore((s) => s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  const me = useStore((s) => s.user)
  const [filter, setFilter] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadMembers(communityId).catch((e) => setError(errorText(e)))
  }, [communityId, loadMembers])

  const canRoles = has(detail.permissions, Permission.MANAGE_ROLES)
  const canKick = has(detail.permissions, Permission.KICK_MEMBERS)
  const canNicks = has(detail.permissions, Permission.MANAGE_NICKNAMES)
  const canOwnNick = canNicks || has(detail.permissions, Permission.CHANGE_NICKNAME)
  const assignable = sortRoles(detail.roles).filter((r) => !r.is_default)
  const roles = new Map(detail.roles.map((r) => [r.id, r]))
  const iAmOwner = detail.community.owner_id === me?.id
  const top = topPosition(detail.roles, members?.find((m) => m.user.id === me?.id)?.role_ids)

  const q = filter.trim().toLowerCase()
  const list = (members ?? [])
    .filter(
      (m) =>
        !q ||
        m.user.username.toLowerCase().includes(q) ||
        displayName(m.user).toLowerCase().includes(q) ||
        !!m.nickname?.toLowerCase().includes(q),
    )
    .sort((a, b) => memberName(a).localeCompare(memberName(b), 'de'))

  async function setRole(m: Member, role: Role, on: boolean) {
    setError(null)
    try {
      if (on) await endpoints.addMemberRole(communityId, m.user.id, role.id)
      else await endpoints.removeMemberRole(communityId, m.user.id, role.id)
      await loadMembers(communityId)
    } catch (e) {
      setError(errorText(e))
    }
  }

  function kick(m: Member) {
    confirmAction({
      title: `„${memberName(m)}“ entfernen`,
      message: `${memberName(m)} wird aus „${detail.community.name}“ entfernt und kann nur mit einer neuen Einladung wieder beitreten.`,
      confirmLabel: 'Entfernen',
      action: async () => {
        await endpoints.removeMember(communityId, m.user.id)
        await loadMembers(communityId)
      },
    })
  }

  return (
    <SettingsPage title={`Mitglieder${members ? ` – ${members.length}` : ''}`}>
      <label className="search-input">
        <Search size={15} aria-hidden="true" />
        <input type="search" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Mitglieder suchen" aria-label="Mitglieder suchen" />
      </label>
      {error && <p className="error">{error}</p>}
      {!members && !error && <p className="muted">Lade …</p>}
      <ul className="admin-rows">
        {list.map((m) => {
          const isOwner = m.user.id === detail.community.owner_id
          const memberRoles = m.role_ids.map((id) => roles.get(id)).filter((r): r is Role => !!r && !r.is_default)
          const open = expanded === m.user.id
          return (
            <li key={m.user.id} className="admin-row">
              <div className="admin-row-main">
                <Avatar user={m.user} name={memberName(m)} size={32} />
                <div className="admin-row-text">
                  <span className="truncate">
                    <strong>{memberName(m)}</strong> <span className="muted small">@{m.user.username}</span>
                    {isOwner && (
                      <span className="member-owner" title="Besitzer">
                        {' '}
                        <Crown size={14} />
                      </span>
                    )}
                  </span>
                  <span className="role-chips">
                    {memberRoles.map((r) => (
                      <span key={r.id} className="role-chip">
                        <span className="role-dot" style={{ background: roleColor(r.color) }} aria-hidden="true" />
                        {r.name}
                      </span>
                    ))}
                  </span>
                </div>
                <div className="admin-row-actions">
                  {canRoles && assignable.length > 0 && (
                    <button
                      type="button"
                      className="button small-button"
                      aria-expanded={open}
                      onClick={() => setExpanded(open ? null : m.user.id)}
                    >
                      Rollen {open ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                    </button>
                  )}
                  {(m.user.id === me?.id ? canOwnNick : canNicks && !isOwner) && (
                    <button
                      type="button"
                      className="button small-button"
                      onClick={() => openDialog({ kind: 'nickname', communityId, userId: m.user.id })}
                    >
                      Nickname
                    </button>
                  )}
                  {canKick && !isOwner && m.user.id !== me?.id && (
                    <button type="button" className="button danger small-button" onClick={() => kick(m)}>
                      Entfernen
                    </button>
                  )}
                </div>
              </div>
              {open && (
                <div className="admin-row-roles">
                  {assignable.map((r) => {
                    const ranked = outranks(r, iAmOwner, top)
                    const allowed = ranked && covers(detail.permissions, r.permissions)
                    const reason = !ranked
                      ? 'Diese Rolle steht nicht unter deiner höchsten Rolle.'
                      : 'Diese Rolle hat Rechte, die du nicht besitzt.'
                    return (
                      <label key={r.id} className={`checkbox${allowed ? '' : ' disabled'}`} title={allowed ? undefined : reason}>
                        <input
                          type="checkbox"
                          checked={m.role_ids.includes(r.id)}
                          disabled={!allowed}
                          onChange={(e) => void setRole(m, r, e.target.checked)}
                        />
                        <span className="role-dot" style={{ background: roleColor(r.color) }} aria-hidden="true" />
                        {r.name}
                      </label>
                    )
                  })}
                </div>
              )}
            </li>
          )
        })}
      </ul>
    </SettingsPage>
  )
}

// ---------------------------------------------------------------------------
// Einladungen
// ---------------------------------------------------------------------------

function InvitesPage({ detail }: { detail: CommunityDetail }) {
  const communityId = detail.community.id
  const publicUrl = useStore((s) => s.instance?.public_url) ?? location.origin
  const [invites, setInvites] = useState<Invite[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const canCreate = has(detail.permissions, Permission.CREATE_INVITES)

  const load = useCallback(() => {
    endpoints
      .invites(communityId)
      .then((list) => {
        setInvites(list)
        setError(null)
      })
      .catch((e) => setError(errorText(e)))
  }, [communityId])

  useEffect(load, [load])

  function revoke(code: string) {
    confirmAction({
      title: 'Einladung widerrufen',
      message: `Der Link mit dem Code „${code}“ funktioniert danach nicht mehr.`,
      confirmLabel: 'Widerrufen',
      action: async () => {
        await endpoints.deleteInvite(code)
        setInvites((list) => list?.filter((i) => i.code !== code) ?? null)
      },
    })
  }

  const base = publicUrl.replace(/\/$/, '')

  return (
    <SettingsPage title="Einladungen" description="Aktive Einladungslinks dieser Community.">
      <div className="form-actions form-actions-start">
        {canCreate && (
          <button type="button" className="button primary small-button" onClick={() => openDialog({ kind: 'invite', communityId })}>
            <UserPlus size={14} aria-hidden="true" /> Einladung erstellen
          </button>
        )}
        <button type="button" className="button small-button" onClick={load}>
          Aktualisieren
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {invites && invites.length === 0 && <p className="muted">Keine aktiven Einladungen.</p>}
      {!invites && !error && <p className="muted">Lade …</p>}
      <ul className="session-list">
        {invites?.map((i) => (
          <li key={i.code} className="session">
            <div className="session-text">
              <code className="truncate">
                {base}/invite/{i.code}
              </code>
              <span className="muted small">
                {i.max_uses == null ? `${i.uses} Nutzungen` : `${i.uses} von ${i.max_uses} Nutzungen`} ·{' '}
                {i.expires_at ? `läuft ab ${dateFmt.format(new Date(i.expires_at))}` : 'läuft nie ab'} · erstellt{' '}
                {dateFmt.format(new Date(i.created_at))}
              </span>
            </div>
            <button type="button" className="button danger small-button" onClick={() => revoke(i.code)}>
              Widerrufen
            </button>
          </li>
        ))}
      </ul>
    </SettingsPage>
  )
}

// ---------------------------------------------------------------------------
// Gefahrenzone (nur Besitzer)
// ---------------------------------------------------------------------------

function DangerPage({ detail, onDone }: { detail: CommunityDetail; onDone: () => void }) {
  const forgetCommunity = useStore((s) => s.forgetCommunity)
  const { community } = detail
  const [confirmName, setConfirmName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function remove(e: FormEvent) {
    e.preventDefault()
    if (confirmName !== community.name) return
    setBusy(true)
    setError(null)
    try {
      await endpoints.deleteCommunity(community.id)
      onDone()
      forgetCommunity(community.id)
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }

  return (
    <SettingsPage title="Gefahrenzone" description="Diese Aktionen betreffen die ganze Community und lassen sich nicht rückgängig machen.">
      <TransferBox detail={detail} onDone={onDone} />
      <SettingsSection
        title="Community löschen"
        danger
        description="Löscht die Community mit allen Kanälen, Nachrichten, Rollen und Einladungen endgültig."
      >
        <form className="form danger-box" onSubmit={remove}>
          <label className="field">
            <span>
              Zur Bestätigung den Namen eingeben: <em>{community.name}</em>
            </span>
            <input value={confirmName} onChange={(e) => setConfirmName(e.target.value)} autoComplete="off" />
          </label>
          {error && <p className="error">{error}</p>}
          <div className="form-actions">
            <button type="submit" className="button danger-solid" disabled={busy || confirmName !== community.name}>
              Community endgültig löschen
            </button>
          </div>
        </form>
      </SettingsSection>
    </SettingsPage>
  )
}

/** Besitz an ein anderes Mitglied übertragen (nur Besitzer). Danach ist man normales Mitglied. */
function TransferBox({ detail, onDone }: { detail: CommunityDetail; onDone: () => void }) {
  const communityId = detail.community.id
  const members = useStore((s) => s.members[communityId])
  const loadMembers = useStore((s) => s.loadMembers)
  const updateCommunity = useStore((s) => s.updateCommunity)
  const me = useStore((s) => s.user)
  const [target, setTarget] = useState('')
  const [msg, setMsg] = useState<Feedback>(null)

  useEffect(() => {
    if (!useStore.getState().members[communityId]) loadMembers(communityId).catch(() => {})
  }, [communityId, loadMembers])

  const candidates = (members ?? [])
    .filter((m) => m.user.id !== me?.id)
    .sort((a, b) => memberName(a).localeCompare(memberName(b), 'de'))
  const chosen = candidates.find((m) => m.user.id === target)

  function transfer() {
    if (!chosen) return
    setMsg(null)
    confirmAction({
      title: 'Besitz übertragen',
      message: `${memberName(chosen)} wird Besitzer von „${detail.community.name}“ und hat dann alle Rechte. Du bleibst normales Mitglied mit deinen Rollen.`,
      confirmLabel: 'Übertragen',
      action: async () => {
        updateCommunity(await endpoints.transferCommunity(communityId, chosen.user.id))
        onDone()
      },
    })
  }

  return (
    <SettingsSection
      title="Besitz übertragen"
      description="Das gewählte Mitglied wird Besitzer und hat alle Rechte. Nötig, bevor du dein Konto löschen kannst."
    >
      {candidates.length === 0 ? (
        <p className="muted small">Es gibt noch keine anderen Mitglieder.</p>
      ) : (
        <div className="copy-row">
          <select value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Neuer Besitzer">
            <option value="">– Mitglied wählen –</option>
            {candidates.map((m) => (
              <option key={m.user.id} value={m.user.id}>
                {memberName(m)} (@{m.user.username})
              </option>
            ))}
          </select>
          <button type="button" className="button danger" disabled={!chosen} onClick={transfer}>
            Übertragen …
          </button>
        </div>
      )}
      {msg && <p className={msg.ok ? 'success' : 'error'}>{msg.text}</p>}
    </SettingsSection>
  )
}
