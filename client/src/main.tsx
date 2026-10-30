import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
// Selbst gehostete Variable Fonts (OFL) – kein Google-Fonts-CDN, auch offline im Desktop-Bundle.
import '@fontsource-variable/inter/wght.css'
import '@fontsource-variable/bricolage-grotesque/wght.css'
import './index.css'
import App from './App.tsx'
import { installDiagnostics } from './lib/diagnostics'
import { installLongPress, installViewportTracking } from './lib/mobile'
import { installScrollbarAutoHide } from './lib/scrollbars'
// Globales Push-to-Talk (nur Desktop-Bundle; registriert sich am Voice-Store).
import './lib/globalPtt'
// Desktop-Hülle: Titelleiste, Schließen ins Tray, Tray-Menü (nur im Tauri-Fenster).
import './lib/desktop'
// Soundeffekte (beobachtet Voice-, App- und Updater-Store).
import './lib/soundEvents'
// Anrufe in DMs: Klingelton, Freizeichen, Benachrichtigung.
import './lib/callAlerts'

// Letzte Client-Fehler im Speicher halten – gesendet nur beim Melden mit Opt-in (Paket K).
installDiagnostics()
// Mobil: langer Druck öffnet Kontextmenüs, sichtbare Höhe folgt der Bildschirmtastatur.
installLongPress()
installViewportTracking()
installScrollbarAutoHide()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
