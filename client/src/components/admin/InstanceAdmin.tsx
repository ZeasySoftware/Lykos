import { Suspense } from 'react'
import { Shield } from 'lucide-react'
import { lazyComponent } from '../../lib/lazy'
import { useStore } from '../../store'
import { LazyLayerFallback } from '../LazyFallback'
import { useAdminPanel } from './adminPanel'

/** Die Verwaltung selbst (alle Reiter) ist ein eigener Chunk – nur Instanz-Admins brauchen sie. */
const InstanceAdmin = lazyComponent(() => import('./InstanceAdminPanel').then((m) => m.InstanceAdmin))

/**
 * Instanz-Verwaltung als Vollbild-Ebene über der App (nur für Instanz-Admins). Wird einmal in der Shell
 * eingehängt und über `useAdminPanel().show()` geöffnet.
 */
export function InstanceAdminLayer() {
  const open = useAdminPanel((s) => s.open)
  const isAdmin = useStore((s) => !!s.user?.is_instance_admin)
  if (!open || !isAdmin) return null
  return (
    <Suspense fallback={<LazyLayerFallback />}>
      <InstanceAdmin />
    </Suspense>
  )
}

/** Schild-Knopf für die Community-Leiste – nur für Instanz-Admins sichtbar. */
export function InstanceAdminRailButton() {
  const isAdmin = useStore((s) => !!s.user?.is_instance_admin)
  const open = useAdminPanel((s) => s.open)
  const show = useAdminPanel((s) => s.show)
  if (!isAdmin) return null
  return (
    <button
      type="button"
      className={`rail-item ia-rail-button${open ? ' active' : ''}`}
      title="Instanz-Verwaltung"
      aria-label="Instanz-Verwaltung"
      onClick={() => show()}
    >
      <Shield size={20} />
    </button>
  )
}

/** Eintrag „Instanz-Verwaltung“ für die Einstellungs-Navigation – nur für Instanz-Admins sichtbar. */
export function InstanceAdminSettingsEntry({ onOpen }: { onOpen?: () => void }) {
  const isAdmin = useStore((s) => !!s.user?.is_instance_admin)
  const show = useAdminPanel((s) => s.show)
  if (!isAdmin) return null
  return (
    <button
      type="button"
      className="ia-settings-entry"
      onClick={() => {
        onOpen?.()
        show()
      }}
    >
      <Shield size={14} aria-hidden="true" />
      Instanz-Verwaltung
    </button>
  )
}
