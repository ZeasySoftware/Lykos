import { LykosMark } from './Brand'
import { percentOf, splashText, type SplashState } from '../lib/updateFlow'

/**
 * Start-Splash der Desktop-App: Wolfslogo, Statuszeile und – beim Update – Fortschrittsbalken.
 * Ohne `state` zeigt er nur „Lykos wird geladen …“ (Anmeldung wird geprüft), damit Update-Prüfung und
 * Laden ohne Bildwechsel ineinander übergehen.
 */
export function Splash({ state }: { state?: SplashState }) {
  const current: SplashState = state ?? { phase: 'done' }
  const percent = current.phase === 'downloading' ? percentOf(current.downloaded, current.total) : null
  const indeterminate = current.phase === 'checking' || (current.phase === 'downloading' && percent === null)
  const width = current.phase === 'installing' ? 100 : (percent ?? 0)

  return (
    <div className="boot-splash" role="status" aria-live="polite">
      <div className="boot-splash-mark">
        <LykosMark />
      </div>
      <div className="boot-splash-name">Lykos</div>
      <div className="boot-splash-status">{splashText(current)}</div>
      <div className={`boot-splash-bar${current.phase === 'done' ? ' is-hidden' : ''}`} aria-hidden="true">
        <div
          className={indeterminate ? 'indeterminate' : undefined}
          style={indeterminate ? undefined : { width: `${width}%` }}
        />
      </div>
    </div>
  )
}
