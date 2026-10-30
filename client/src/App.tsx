import { Suspense, useEffect, useState } from 'react'
import { AuthScreen } from './components/AuthScreen'
import { LykosMark } from './components/Brand'
import { AccountDock, ChannelSidebar } from './components/ChannelSidebar'
import { CommunityRail } from './components/CommunityRail'
import { AcceptInviteDialog } from './components/Dialogs'
import { DmSidebar } from './components/DirectMessages'
import { FriendsView } from './components/FriendsView'
import { useFriends } from './lib/friendsStore'
import { SearchPanel } from './components/SearchPanel'
import { DownloadPage } from './components/DownloadPage'
import { LegalPage } from './components/LegalPage'
import { VerifyEmailPage } from './components/AccountRecoveryPages'
import { OidcResultNotice } from './components/settings/OidcResultNotice'
import { Splash } from './components/Splash'
import { Titlebar } from './components/Titlebar'
import { CloseAppHost } from './components/CloseAppDialog'
import { hasDesktopShell } from './lib/desktop'
import { runStartupUpdate, useUpdater } from './lib/updater'
import { FeedbackHost } from './components/feedback/FeedbackHub'
import { AnnouncementBanner } from './components/admin/AnnouncementBanner'
import { InstanceAdminLayer } from './components/admin/InstanceAdmin'
import { ResetPasswordPage } from './components/admin/ResetPasswordPage'
import { resetTokenFromLocation } from './lib/adminApi'
import { isDesktopBundle } from './lib/api'
import { DialogHost, Toasts } from './components/DialogHost'
import { MemberList } from './components/MemberList'
import { MenuHost } from './components/Menu'
import { openDialog, useDialogs } from './lib/dialogs'
import { useMenu } from './lib/menuStore'
import { useAdminPanel } from './components/admin/adminPanel'
import { useProfileModal } from './components/customize/profileModal'
import { MessageView } from './components/MessageView'
import { ProfileCard } from './components/ProfileCard'
import { ProfileModalHost } from './components/customize/ProfileView'
import { ServerSetup } from './components/ServerSetup'
import { useIdleDetection } from './lib/presence'
import { lazyComponent, preloadLazyPanels } from './lib/lazy'
import { IncomingCalls } from './components/DmCall'
import type { LegalKind, MessageChannel } from './lib/api'
import { Permission } from './lib/permissions'
import { dmTitle, useStore } from './store'
import { Menu, Users } from 'lucide-react'
import { DrawerButton } from './components/MobileNav'
import { useEscapeLayer } from './lib/layers'
import { useDrawerSwipe, useIsMobile, useMobileNav } from './lib/mobile'

/** Call-Bühne (Kacheln, Layout, Steuerung) erst laden, wenn ein Sprachkanal geöffnet wird. */
const CallStage = lazyComponent(() => import('./components/CallStage').then((m) => m.CallStage))

const MEMBERS_KEY = 'lykos_members_open'
/** Rechte in DMs (wie auf dem Server): lesen, schreiben, Dateien anhängen. */
const DM_PERMISSIONS = Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES | Permission.ATTACH_FILES

/** Öffentliche Download-Seite (`/download`) – nur im Web, die Desktop-App braucht sie nicht. */
const downloadRoute = !isDesktopBundle && /^\/download\/?$/.test(location.pathname)
/** Passwort-Reset-Link (`/reset-password#token=…`, vom Admin oder per Mail) – nur im Web. */
const resetToken = isDesktopBundle ? null : resetTokenFromLocation()
/** Bestätigungslink aus der Mail (`/verify-email#token=…`) – öffnet sich immer im Browser. */
const verifyRoute = !isDesktopBundle && /^\/verify-email\/?$/.test(location.pathname)
/** Rechtstexte der Instanz (`/impressum`, `/datenschutz`) – nur im Web; die Desktop-App öffnet sie im Browser. */
const legalRoute = isDesktopBundle ? null : legalKindFromPath(location.pathname)
const publicRoute = downloadRoute || resetToken !== null || verifyRoute || legalRoute !== null

function legalKindFromPath(path: string): LegalKind | null {
  if (/^\/impressum\/?$/.test(path)) return 'imprint'
  if (/^\/datenschutz\/?$/.test(path)) return 'privacy'
  return null
}

function readMembersOpen(): boolean {
  try {
    return localStorage.getItem(MEMBERS_KEY) !== '0'
  } catch {
    return true
  }
}

/** Desktop-Fenster (ohne Rahmen): eigene Titelleiste über allem; im Browser unverändert. */
export default function App() {
  return (
    <>
      {hasDesktopShell && <Titlebar />}
      <AppContent />
      {hasDesktopShell && <CloseAppHost />}
    </>
  )
}

function AppContent() {
  const init = useStore((s) => s.init)
  const authChecked = useStore((s) => s.authChecked)
  const user = useStore((s) => s.user)
  const pendingInvite = useStore((s) => s.pendingInvite)
  const serverChosen = useStore((s) => s.serverChosen)
  // Desktop-Bundle: erst Update-Prüfung im Splash (ggf. Update installieren und neu starten), dann die App.
  const booting = useUpdater((s) => s.booting)
  const splash = useUpdater((s) => s.splash)

  useEffect(() => {
    void runStartupUpdate()
  }, [])

  useEffect(() => {
    // Öffentliche Seiten (Download, Rechtstexte, Reset- und Bestätigungslinks) brauchen weder Anmeldung noch Gateway;
    // während der Update-Prüfung noch nicht verbinden (die App startet ggf. gleich neu).
    if (!publicRoute && !booting) void init()
  }, [init, booting])

  if (downloadRoute) return <DownloadPage />
  if (legalRoute) return <LegalPage kind={legalRoute} />
  if (resetToken !== null) return <ResetPasswordPage token={resetToken} />
  if (verifyRoute) return <VerifyEmailPage />
  if (booting) return <Splash state={splash} />

  return (
    <Screen
      serverChosen={serverChosen}
      authChecked={authChecked}
      loggedIn={!!user}
      pendingInvite={pendingInvite}
    />
  )
}

function Screen({
  serverChosen,
  authChecked,
  loggedIn,
  pendingInvite,
}: {
  serverChosen: boolean
  authChecked: boolean
  loggedIn: boolean
  pendingInvite: string | null
}) {
  // Nach dem Abmelden keine Ebenen (Einstellungen, Verwaltung, Profil, Menüs) in die nächste Sitzung mitnehmen.
  useEffect(() => {
    if (loggedIn) return
    useDialogs.setState({ stack: [] })
    useMenu.getState().close(false)
    useAdminPanel.getState().hide()
    useProfileModal.getState().close()
  }, [loggedIn])

  // Nachgeladene Bereiche (Einstellungen, Verwaltung, Call-Bühne …) im Leerlauf schon holen.
  useEffect(() => {
    if (loggedIn) preloadLazyPanels()
  }, [loggedIn])

  // Desktop-Bundle: Instanzwahl vor allem anderen.
  if (!serverChosen) return <ServerSetup />
  if (!authChecked) return isDesktopBundle ? <Splash /> : <div className="splash">Lykos wird geladen …</div>
  if (!loggedIn) return <AuthScreen />

  return (
    <>
      <div className="ia-app-frame">
        <AnnouncementBanner />
        <Shell />
      </div>
      <InstanceAdminLayer />
      {pendingInvite && <AcceptInviteDialog code={pendingInvite} />}
      <DialogHost />
      <MenuHost />
      <Toasts />
      <FeedbackHost />
    </>
  )
}

function Shell() {
  const selectedCommunityId = useStore((s) => s.selectedCommunityId)
  const selectedChannelId = useStore((s) => s.selectedChannelId)
  const dmView = useStore((s) => s.dmView)
  const communityDetail = useStore((s) => (selectedCommunityId ? s.details[selectedCommunityId] : undefined))
  const detail = dmView ? undefined : communityDetail
  const dm = useStore((s) => (s.dmView && s.selectedDmId ? s.dms[s.selectedDmId] : undefined))
  const me = useStore((s) => s.user)
  // 1:1-DM mit einer blockierten Person: nur lesen (der Server erzwingt das auch in die Gegenrichtung).
  const dmPartner = dm?.kind === 'dm' ? dm.recipients.find((u) => u.id !== me?.id) : undefined
  const partnerBlocked = useFriends((s) => !!dmPartner && s.relationships[dmPartner.id]?.type === 'blocked')
  const search = useStore((s) => s.search)
  const searchOpen = search !== null
  const [membersOpen, setMembersOpen] = useState(readMembersOpen)
  // Mobil: eine Spalte; Rail + Kanalliste als Schublade, Mitglieder als Vollbild-Ebene.
  const mobile = useIsMobile()
  const drawerOpen = useMobileNav((s) => s.drawerOpen) && mobile
  const closeDrawer = useMobileNav((s) => s.closeDrawer)
  const mobileMembers = useMobileNav((s) => s.membersOpen)
  const setMobileMembers = useMobileNav((s) => s.setMembersOpen)
  useDrawerSwipe(mobile)
  useEscapeLayer(drawerOpen, closeDrawer)
  // Präsenz: nach 10 min ohne Eingabe automatisch abwesend.
  useIdleDetection(useStore((s) => s.setAutoIdle))
  // Strg+, öffnet die Einstellungen (wie bei Discord).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === ',') {
        e.preventDefault()
        openDialog({ kind: 'settings' })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const textChannel = detail?.channels.find((c) => c.id === selectedChannelId && c.kind === 'text')
  // Sprachkanal ausgewählt → Call-Bühne (bzw. Vorschau mit „Beitreten“).
  const voiceChannel = detail?.channels.find((c) => c.id === selectedChannelId && c.kind === 'voice')
  // DMs nutzen dieselbe Nachrichtenansicht; der Name ist der Anzeigename der DM.
  const channel: MessageChannel | undefined = dm
    ? { id: dm.id, community_id: null, name: dmTitle(dm, me?.id), kind: dm.kind, topic: null, position: 0 }
    : textChannel && { ...textChannel, kind: 'text' }
  const channelPerms = dm
    ? partnerBlocked
      ? Permission.VIEW_CHANNEL
      : DM_PERMISSIONS
    : textChannel
      ? (detail?.channel_permissions[textChannel.id] ?? detail?.permissions ?? 0)
      : 0
  const showMembers = !!detail && (mobile ? mobileMembers : membersOpen) && !searchOpen
  useEscapeLayer(mobile && showMembers, () => setMobileMembers(false))

  function toggleMembers() {
    if (mobile) {
      setMobileMembers(!mobileMembers)
      return
    }
    setMembersOpen((open) => {
      try {
        localStorage.setItem(MEMBERS_KEY, open ? '0' : '1')
      } catch {
        // egal
      }
      return !open
    })
  }

  return (
    <div className={`shell${searchOpen ? ' with-search' : showMembers ? ' with-members' : ''}`}>
      {/* Desktop: `display: contents` (Rail, Kanalleiste und Konto-Block liegen direkt im Raster).
          Mobil: Schublade von links; ein Kanal- oder DM-Klick schließt sie. */}
      <div
        className={`shell-drawer${drawerOpen ? ' open' : ''}`}
        onClick={(e) => {
          if (mobile && (e.target as HTMLElement).closest('.channel:not(:disabled)')) closeDrawer()
        }}
      >
        <CommunityRail />
        {dmView ? <DmSidebar /> : <ChannelSidebar detail={detail} />}
        <AccountDock />
      </div>
      {drawerOpen && <div className="drawer-backdrop" onClick={closeDrawer} aria-hidden="true" />}
      <main className="content">
        {detail && !searchOpen && (
          <button
            type="button"
            className="members-toggle icon-button"
            title={membersOpen ? 'Mitglieder ausblenden' : 'Mitglieder einblenden'}
            aria-pressed={membersOpen}
            onClick={toggleMembers}
          >
            <Users size={18} />
          </button>
        )}
        {voiceChannel && detail ? (
          <Suspense key={voiceChannel.id} fallback={<div className="call-stage" aria-busy="true" />}>
            <CallStage channel={voiceChannel} detail={detail} />
          </Suspense>
        ) : channel ? (
          <MessageView
            key={channel.id}
            channel={channel}
            permissions={channelPerms}
            readOnlyHint={partnerBlocked ? 'Du hast diese Person blockiert und kannst ihr nicht schreiben.' : undefined}
          />
        ) : dmView ? (
          <FriendsView />
        ) : (
          <EmptyState hasCommunity={!!detail} dmView={dmView} mobile={mobile} />
        )}
      </main>
      {search && <SearchPanel key={`${search.communityId}|${search.channelId}|${search.query}`} />}
      {showMembers && (
        <MemberList
          key={detail.community.id}
          detail={detail}
          onClose={mobile ? () => setMobileMembers(false) : undefined}
        />
      )}
      <ProfileCard />
      <IncomingCalls />
      <OidcResultNotice />
      <ProfileModalHost />
    </div>
  )
}

function EmptyState({ hasCommunity, dmView, mobile }: { hasCommunity: boolean; dmView: boolean; mobile: boolean }) {
  const openDrawer = useMobileNav((s) => s.openDrawer)
  // Mobil gibt es kein „links“ – die Liste liegt in der Schublade.
  const where = mobile ? 'in der Seitenleiste' : 'links'
  return (
    <>
      <header className="pane-header welcome-header mobile-only">
        <DrawerButton />
        <strong className="truncate">{dmView ? 'Direktnachrichten' : 'Lykos'}</strong>
      </header>
      <div className="welcome">
        <LykosMark className="welcome-mark" />
        <h1>{dmView ? 'Direktnachrichten' : 'Willkommen bei Lykos'}</h1>
        <p>
          {dmView
            ? `Wähle ${where} eine Unterhaltung oder starte über das + eine neue.`
            : hasCommunity
            ? `Wähle ${where} einen Textkanal aus.`
            : `Lege über das + ${where} eine Community an oder tritt mit einer Einladung bei.`}
        </p>
        {mobile && (
          <button type="button" className="button primary welcome-open" onClick={openDrawer}>
            <Menu size={18} aria-hidden="true" />
            {dmView ? 'Unterhaltungen anzeigen' : 'Communities und Kanäle'}
          </button>
        )}
      </div>
    </>
  )
}
