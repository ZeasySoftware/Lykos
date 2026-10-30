# Lykos – Projekt

Datenschutzfreundliche, performante Discord-Alternative mit Fokus auf die Kernfunktionen:
Text-Chat, Voice, Filesharing, Streams (Screenshare).

## Motivation

Discord verlangt zunehmend Identitätsnachweise (Alters-/ID-Verifikation) und sammelt viele Daten.
Lykos soll das Gegenmodell sein: **kein Identitätszwang, minimale Datenerhebung, kein Tracking** –
und dabei schneller und ressourcenschonender als der Electron-Client von Discord.

## Eckpunkte (entschieden)

| Thema | Entscheidung |
|---|---|
| Zielgruppe | Kleine, geschlossene Gruppen (Freundeskreise, Vereine, Teams), Zugang per Einladung möglich |
| Skalierung | Architektur horizontal skalierbar auslegen; Umzug auf stärkere Hardware = Deploy-Ziel wechseln, kein Umbau |
| Betriebsmodell | Eine zentrale Instanz pro Gruppe/Betreiber (wie Discord), keine Föderation |
| Backend | Rust (axum + tokio) |
| Medien (Voice/Video/Screenshare) | LiveKit SFU (self-hosted), kein Eigenbau |
| Datenbank | PostgreSQL |
| Dateien | Lokales Verzeichnis oder S3-kompatibler Object Storage (Garage, MinIO, AWS) |
| Clients | Tauri 2 + React/TypeScript (Desktop), dieselbe Codebasis als Web-Client; Mobile später |
| Deployment | Docker Compose auf einem einzelnen Host |
| Lizenz | GNU AGPL-3.0-or-later (siehe `LICENSE`, `NOTICE`) |

## Datenschutz-Grundsätze

- Registrierung mit Benutzername + Passwort; **keine Telefonnummer, keine Ausweis-/Altersverifikation**.
  E-Mail optional (nur für Passwort-Reset und als zweiter Faktor).
- Keine Analytics, keine Drittanbieter-Tracker, keine Telemetrie ohne Opt-in.
- Metadaten minimieren: IP-Adressen nicht dauerhaft speichern, Logs mit kurzer Aufbewahrung.
- Transportverschlüsselung überall (TLS, DTLS-SRTP für Medien).
- Später: Ende-zu-Ende-Verschlüsselung für DMs und Voice; Datenmodell so anlegen, dass E2EE für
  Kanäle (MLS, RFC 9420) nachrüstbar bleibt.

## Architektur-Grundsätze

- **Community als Einheit:** Rollen, Rechte, Aufbewahrung und Kontingente hängen an der Community. Keine globalen
  Annahmen, die eine spätere Organisations-Ebene verhindern.
- **Instanz-Konfiguration statt Hardcoding:** Hostname, Branding, Registrierungsmodus, Limits per Config bzw.
  Instanz-Einstellungen – Betrieb ohne Fork.
- **Auth austauschbar:** Lokale Accounts plus OIDC/OAuth2 (Discord, GitHub, Google, Keycloak, Authentik, …).
- **Feingranulares Rechtesystem** (Bitflags pro Rolle + Kanal-Overrides).
- **Audit-Events** für administrative Aktionen (Community- und Instanz-Audit-Log).
- **Aufbewahrungsregeln und Datenexport** als First-Class-Konzepte.
- **Modulare Feature-Grenzen:** Neue Bereiche als eigene Module/Crates mit eigener API, nicht in den Chat-Kern
  verwoben.
- **API-first:** Clients sprechen nur über die dokumentierte API/Gateway (`docs/api.md`) – Basis für Integrationen
  und Bots.

## Roadmap (grob)

1. **Fundament** – Accounts, Communities/Kanäle/Rollen, Echtzeit-Text-Chat (WebSocket-Gateway), Tauri- + Web-Client,
   Docker-Deployment *(erledigt, 0.1.0)*
2. **Voice** – Voice-Kanäle über LiveKit (PTT, Mute/Deafen, Geräteauswahl) *(erledigt, 0.1.0)*
3. **Dateien** – Uploads, Vorschauen, Limits, S3 *(erledigt, 0.1.0–0.3.0)*
4. **Streams** – Screenshare und Webcam *(erledigt, 0.1.0–0.2.0)*
5. **Privacy-Ausbau** – Datenexport *(erledigt, 0.3.0)*, E2EE für DMs/Voice, Aufbewahrungsregeln
6. **Weitere Ideen** – Freundesystem, GIF-Suche über den Server (ohne Nutzer-IPs beim Anbieter), Mobil-Layout,
   Mobile-Apps

Was sich pro Version geändert hat, steht in `CHANGELOG.md`.
