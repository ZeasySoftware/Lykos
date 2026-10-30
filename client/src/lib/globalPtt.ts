/**
 * Globales Push-to-Talk im Desktop-Client (Tauri-Plugin `global-shortcut`).
 *
 * Solange man in einem Sprachkanal ist und Push-to-Talk mit „global“ gewählt hat, wird die
 * PTT-Taste systemweit registriert – sie wirkt dann auch, wenn Lykos nicht im Vordergrund ist.
 * Das Plugin meldet Drücken und Loslassen getrennt (`Pressed`/`Released`).
 *
 * Achtung: Unter Windows (RegisterHotKey) gehört die Taste währenddessen Lykos allein; andere
 * Programme bekommen sie nicht. Deshalb nur während eines Gesprächs registrieren.
 * Im Browser bleibt es beim Fokus-PTT aus `voice.ts`.
 */
import { isDesktopBundle } from './api'
import { useVoice } from './voice'

/**
 * `KeyboardEvent.code`-Werte, die das Plugin als globale Taste versteht (Namen wie in
 * `global-hotkey`). Reine Modifikatortasten (Strg, Umschalt, Alt) und `IntlBackslash` gehen nicht.
 */
const SUPPORTED =
  /^(Backquote|Backslash|BracketLeft|BracketRight|Pause|Comma|Digit\d|Equal|Key[A-Z]|Minus|Period|Quote|Semicolon|Slash|Backspace|CapsLock|Enter|Space|Tab|Delete|End|Home|Insert|PageDown|PageUp|PrintScreen|ScrollLock|Arrow(Down|Left|Right|Up)|NumLock|Numpad(\d|Add|Decimal|Divide|Enter|Equal|Multiply|Subtract)|F([1-9]|1\d|2[0-4])|AudioVolume(Down|Up|Mute)|Media(Play|Pause|PlayPause|Stop|TrackNext|TrackPrevious))$/

/** Gibt es globales PTT in dieser Umgebung? (Nur im Desktop-Bundle.) */
export const globalPttAvailable = isDesktopBundle

/** Kann diese Taste (`KeyboardEvent.code`) global belegt werden? */
export function isGlobalShortcutKey(code: string): boolean {
  return SUPPORTED.test(code)
}

type Plugin = typeof import('@tauri-apps/plugin-global-shortcut')
let plugin: Promise<Plugin> | null = null
function loadPlugin(): Promise<Plugin> {
  plugin ??= import('@tauri-apps/plugin-global-shortcut')
  return plugin
}

/** Aktuell registrierte Taste. */
let registered: string | null = null
/** Zuletzt fehlgeschlagene Taste. */
let failed: string | null = null
/** Während die Taste in den Einstellungen neu belegt wird, freigeben (sonst kommt sie nicht an). */
let paused = false
/** Registrieren/Freigeben nacheinander ausführen. */
let queue: Promise<void> = Promise.resolve()

function wantedKey(): string | null {
  const v = useVoice.getState()
  if (paused || v.status === 'idle' || v.settings.inputMode !== 'ptt' || !v.settings.pttGlobal) return null
  return isGlobalShortcutKey(v.settings.pttKey) ? v.settings.pttKey : null
}

function sync() {
  queue = queue.then(async () => {
    const want = wantedKey()
    // Neuer Versuch erst nach Auflegen/Umstellen, nicht bei jeder Store-Änderung.
    if (!want) failed = null
    if (want === registered || (want !== null && want === failed)) return
    const gs = await loadPlugin()
    if (registered) {
      await gs.unregister(registered).catch(() => {})
      registered = null
      useVoice.getState().setPttPressed(false)
    }
    if (!want) return
    try {
      await gs.register(want, (e) => useVoice.getState().setPttPressed(e.state === 'Pressed'))
      registered = want
    } catch {
      failed = want
      useVoice.setState({
        error: 'Die Push-to-Talk-Taste ist von einem anderen Programm belegt – sie wirkt nur bei aktivem Fenster.',
      })
    }
  })
}

/** Während des Neubelegens der Taste die globale Registrierung aussetzen. */
export function pauseGlobalPtt(pause: boolean) {
  if (!globalPttAvailable || paused === pause) return
  paused = pause
  sync()
}

if (globalPttAvailable) {
  // Nach einem Neuladen der Oberfläche können noch Registrierungen des alten Fensters bestehen.
  queue = loadPlugin()
    .then((gs) => gs.unregisterAll())
    .catch(() => {})
  useVoice.subscribe((s, prev) => {
    if (s.status !== prev.status || s.settings !== prev.settings) sync()
  })
}
