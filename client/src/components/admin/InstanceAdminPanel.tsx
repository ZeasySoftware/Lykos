import { useEffect } from 'react'
import type { ReactNode } from 'react'
import { Flag, LayoutDashboard, Mail, MessagesSquare, ScrollText, Settings2, Shield, Users, X } from 'lucide-react'
import { useStore } from '../../store'
import { FeedbackAdmin } from '../feedback/FeedbackAdmin'
import { useAdminPanel } from './adminPanel'
import type { AdminTab } from './adminPanel'
import { AuditTab } from './AuditTab'
import { CommunitiesTab } from './CommunitiesTab'
import { InvitesTab } from './InvitesTab'
import { OverviewTab } from './OverviewTab'
import { SettingsTab } from './SettingsTab'
import { UsersTab } from './UsersTab'

// Inhalt der Instanz-Verwaltung – eigener Chunk, geladen über `InstanceAdminLayer` (InstanceAdmin.tsx).

const TABS: { id: AdminTab; label: string; icon: ReactNode }[] = [
  { id: 'overview', label: 'Übersicht', icon: <LayoutDashboard size={16} /> },
  { id: 'invites', label: 'Einladungen', icon: <Mail size={16} /> },
  { id: 'users', label: 'Nutzer', icon: <Users size={16} /> },
  { id: 'communities', label: 'Communities', icon: <MessagesSquare size={16} /> },
  { id: 'feedback', label: 'Meldungen', icon: <Flag size={16} /> },
  { id: 'settings', label: 'Einstellungen', icon: <Settings2 size={16} /> },
  { id: 'audit', label: 'Audit-Log', icon: <ScrollText size={16} /> },
]

export function InstanceAdmin() {
  const tab = useAdminPanel((s) => s.tab)
  const setTab = useAdminPanel((s) => s.setTab)
  const hide = useAdminPanel((s) => s.hide)
  const instanceName = useStore((s) => s.instance?.name ?? 'Lykos')

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Offene Dialoge schließen zuerst sich selbst.
      if (e.key === 'Escape' && !document.querySelector('.modal-backdrop')) hide()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [hide])

  return (
    <div className="ia-layer" role="dialog" aria-modal="true" aria-label="Instanz-Verwaltung">
      <nav className="ia-nav" aria-label="Bereiche der Instanz-Verwaltung">
        <div className="ia-nav-title">
          <Shield size={18} aria-hidden="true" />
          <span>
            <strong>Instanz-Verwaltung</strong>
            <span className="muted small truncate">{instanceName}</span>
          </span>
        </div>
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            className={tab === t.id ? 'active' : ''}
            aria-current={tab === t.id}
            onClick={() => setTab(t.id)}
          >
            {t.icon}
            {t.label}
          </button>
        ))}
      </nav>
      <main className="ia-content">
        <div className="ia-content-inner">
          {tab === 'overview' && <OverviewTab />}
          {tab === 'invites' && <InvitesTab />}
          {tab === 'users' && <UsersTab />}
          {tab === 'communities' && <CommunitiesTab />}
          {tab === 'feedback' && <FeedbackAdmin />}
          {tab === 'settings' && <SettingsTab />}
          {tab === 'audit' && <AuditTab />}
        </div>
        <button type="button" className="ia-close" onClick={hide} aria-label="Instanz-Verwaltung schließen">
          <X size={20} />
          <span className="settings-close-hint" aria-hidden="true">
            ESC
          </span>
        </button>
      </main>
    </div>
  )
}
