import { useState } from 'react'
import type { PresenceStatus } from '../lib/api'
import { avatarColor, initials, useAvatarSrc } from '../lib/avatar'
import { buildSelfMenu } from '../lib/contextMenus'
import { useDropdown } from '../lib/menuStore'
import { presenceLabel, statusOf } from '../lib/presence'
import { displayName, useStore } from '../store'

/** Farbiger Statuspunkt, z. B. unten rechts am Avatar. */
export function PresenceDot({ status, className = '' }: { status: PresenceStatus; className?: string }) {
  return (
    <span
      className={`presence-dot presence-${status} ${className}`.trim()}
      title={presenceLabel[status]}
      aria-label={presenceLabel[status]}
    />
  )
}

/**
 * Runder Avatar: Profilbild, sonst Initialen auf einer aus der Nutzer-ID abgeleiteten Farbe;
 * optional mit Statuspunkt. `name` ist der angezeigte Name (Nickname > Anzeigename > Benutzername).
 */
export function Avatar({
  user,
  name,
  size = 32,
  status,
  className = '',
}: {
  user: { id: string; avatar_url?: string | null } | null
  name: string
  size?: number
  status?: PresenceStatus
  className?: string
}) {
  const src = useAvatarSrc(user?.avatar_url)
  const [failed, setFailed] = useState<string | null>(null)
  const showImage = src && failed !== src
  return (
    <span className={`avatar ${className}`.trim()} style={{ width: size, height: size }}>
      {showImage ? (
        <img className="avatar-img" src={src} alt="" draggable={false} onError={() => setFailed(src)} />
      ) : (
        <span
          className="avatar-initial"
          aria-hidden="true"
          style={{ background: avatarColor(user?.id ?? name), fontSize: Math.max(9, Math.round(size * 0.38)) }}
        >
          {initials(name)}
        </span>
      )}
      {status && <PresenceDot status={status} className="avatar-dot" />}
    </span>
  )
}

/** Eigener Avatar + Name im Nutzerbereich unten links; Klick öffnet das Konto-Menü (Status, Profil, Abmelden). */
export function StatusPicker() {
  const user = useStore((s) => s.user)
  const gatewayStatus = useStore((s) => s.gatewayStatus)
  const chosen = useStore((s) => s.chosenStatus)
  const status = useStore((s) => (s.user ? statusOf(s, s.user.id) : 'offline'))
  const dropdown = useDropdown(buildSelfMenu, { label: 'Status und Konto', placement: 'above' })

  const label =
    gatewayStatus === 'connecting'
      ? 'Verbinde …'
      : gatewayStatus === 'offline'
        ? 'Offline'
        : presenceLabel[chosen === 'invisible' ? 'invisible' : status]

  return (
    <div className="status-picker">
      <button type="button" className="status-picker-button" title="Status und Konto" {...dropdown}>
        <Avatar user={user} name={displayName(user)} status={status} />
        <span className="user-panel-info">
          <span className="truncate user-panel-name">{displayName(user)}</span>
          <span className="truncate status-picker-label" title={label}>
            {gatewayStatus === 'online' && user?.status_text ? user.status_text : label}
          </span>
        </span>
      </button>
    </div>
  )
}
