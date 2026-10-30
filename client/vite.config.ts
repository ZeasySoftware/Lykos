import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Tauri erwartet einen festen Port und soll seine Ausgaben nicht weggeräumt bekommen.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // Im Web-Dev-Modus die API auf den lokalen Rust-Server durchreichen (Ziel per `LYKOS_DEV_API` änderbar).
    // Das Gateway nimmt nur Browser-Verbindungen vom eigenen Origin (`LYKOS_PUBLIC_URL`) bzw. aus
    // `LYKOS_CORS_ORIGINS` an – der Proxy gibt sich deshalb als Ziel-Origin aus.
    proxy: {
      '/api': {
        target: process.env.LYKOS_DEV_API || 'http://localhost:8080',
        ws: true,
        headers: { origin: process.env.LYKOS_DEV_API || 'http://localhost:8080' },
      },
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  define: {
    __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? 'dev'),
  },
})
