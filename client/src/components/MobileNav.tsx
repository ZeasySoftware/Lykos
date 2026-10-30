import { Menu } from 'lucide-react'
import { useMobileNav } from '../lib/mobile'

/**
 * Hamburger im Kopf von Chat, Call und Startseite: öffnet im Mobil-Layout die Schublade mit
 * Community-Rail und Kanalliste. Auf dem Desktop per CSS (`.mobile-only`) ausgeblendet.
 */
export function DrawerButton() {
  const openDrawer = useMobileNav((s) => s.openDrawer)
  return (
    <button
      type="button"
      className="icon-button drawer-button mobile-only"
      aria-label="Communities und Kanäle öffnen"
      title="Communities und Kanäle"
      onClick={openDrawer}
    >
      <Menu size={20} />
    </button>
  )
}
