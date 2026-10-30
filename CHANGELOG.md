# Changelog

Alle nennenswerten Änderungen an Lykos stehen in dieser Datei.

Das Format orientiert sich an [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), die Versionierung an
[Semantic Versioning](https://semver.org/lang/de/) (in der 0.x-Phase: Minor = neue Funktionen bzw. Migrationen,
Patch = Fehlerbehebungen). Migrationen laufen beim Serverstart automatisch.

## [Unveröffentlicht]

### Hinzugefügt

- Lizenz **AGPL-3.0-or-later** (`LICENSE`, `NOTICE`), Lizenzfelder in `Cargo.toml` und `package.json`.
- `THIRD_PARTY_NOTICES.md` mit geprüften Drittlizenzen.
- `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), Issue- und PR-Vorlagen für
  Gitea und GitHub.
- CI-Workflow für GitHub (`.github/workflows/ci.yml`).

### Geändert

- README für Selbst-Hoster neu gegliedert (Docker Compose, Reverse Proxy, LiveKit, Umgebungsvariablen, S3).
- Gitea-CI wählt den Runner über die Variable `LYKOS_RUNNER` (Standard `ubuntu-latest`), damit sie in Forks läuft.
- `deploy/deploy.sh` hat kein fest eingetragenes Ziel mehr: Argument oder `LYKOS_DEPLOY_TARGET`.
- `deploy/.env.example` mit neutralen Beispielwerten (`chat.example.org`).

## [0.4.1] – 2026-10-29

### Behoben

- Community-Banner überdeckte Icon und Namen im Community-Dropdown.

## [0.4.0] – 2026-10-29

### Hinzugefügt

- **Konto-Sicherheit:** optionale, bestätigte E-Mail-Adresse; „Passwort vergessen“ per Mail; Zwei-Faktor-
  Anmeldung (TOTP, E-Mail-Code, Wiederherstellungscodes); Anmeldung mit Discord, GitHub, Google sowie generischem
  OIDC/OAuth2 (in der Desktop-App vorerst ausgeblendet). Migrationen 0012, 0017.
- **Instanz-Verwaltung:** Instanz-Einladungen, Nutzer sperren/zum Admin machen/Reset-Link/löschen,
  Laufzeit-Einstellungen, Dashboard, Community-Übersicht, Ankündigungs-Banner, Instanz-Audit-Log. Migration 0013.
- **Custom-Emotes** pro Community (Recht `MANAGE_EMOJIS`), überall für Mitglieder nutzbar; vollständiger
  Emoji-Picker, Emoji-Button an Textfeldern, `:shortcode:`-Vervollständigung. Migration 0014.
- **Gestaltung:** Community-Icon, -Banner, -Beschreibung, -Akzentfarbe, Systemkanal mit Beitrittsmeldung;
  Profilbanner, Profilfarbe, Pronomen, großes Profil. Migration 0015.
- **Feedback-Tool** (Strg+Umschalt+F): Fehler und Vorschläge mit Screenshot und optionalen Technikinfos,
  Abstimmung über Vorschläge, Admin-Ansicht, optional als Gitea-Issue. Migration 0016.
- **Call-Bühne:** Klick auf einen Sprachkanal öffnet Kacheln mit Sprech-Rahmen, Fokus, Vollbild, Steuerleiste und
  Mini-Ansicht; Kameras werden nur bei sichtbarer Kachel abonniert.
- **Sounds:** eigene, per Web Audio synthetisierte Klangfamilie, pro Ereignis schaltbar.
- Kontextmenüs überall (mit Untermenüs und Tastaturbedienung), Community-Dropdown.

### Geändert

- **Desktop-Updater wie bei Discord:** Splash beim Start lädt und installiert Updates still (NSIS `quiet`, ohne
  UAC); im laufenden Betrieb Hintergrund-Download und grüner Pfeil „Neustart zum Aktualisieren“ (nie im Anruf).
- WebView2 erlaubt Mikrofon und Kamera der eigenen App ohne Rückfrage.
- Einstellungen als Vollbild-Ebene mit Leiste für ungespeicherte Änderungen; Nutzerleiste über Rail und Sidebar.

## [0.3.1] – 2026-10-29

### Geändert

- Desktop-Updater: Splash beim Start und stiller Hintergrund-Download (erste Version des neuen Ablaufs).

## [0.3.0] – 2026-10-29

### Hinzugefügt

- **Nachrichten:** Antworten, Emoji-Reaktionen (`ADD_REACTIONS`), @Erwähnungen inkl. `@everyone`
  (`MENTION_EVERYONE`) mit Erwähnungszähler, Markdown (eigener Parser, kein HTML), angepinnte Nachrichten.
  Migration 0008.
- **Konto & Profil:** Avatare (clientseitig auf 256 px skaliert, EXIF entfernt), Profilkarte, Über-mich/Status,
  Nicknames (`CHANGE_NICKNAME`, `MANAGE_NICKNAMES`), Passwort ändern, alle Sitzungen abmelden, Datenexport (JSON),
  Konto löschen, Community-Besitz übertragen. Migration 0009.
- **Direktnachrichten:** 1:1 und Gruppen bis 10 Personen (nur mit gemeinsamer Community, abschaltbar);
  Volltextsuche in Communities und DMs mit Sprung zur Nachricht. Migration 0010.
- **Voice & Desktop:** serverseitiger Entzug von Sprechen/Streamen/Video im laufenden Anruf, globales
  Push-to-Talk (Desktop), Auto-Update der Desktop-App (Manifest über `/api/desktop/latest.json`), Download-Seite
  `/download`.
- Rollenhierarchie mit Drag-and-drop-Sortierung, Upload-Kontingente pro Nutzer (`LYKOS_USER_QUOTA_MB`) und
  Community, S3-Speicher für Anhänge (`LYKOS_STORAGE=s3`). Migration 0011.

### Geändert

- Neues Design: kühle Nachtgrau-Flächen, Akzent-Violett nur für Aktionen/Auswahl/Fokus, selbst gehostete Schriften
  Inter und Bricolage Grotesque, Wortmarke.
- Neues Logo: Wolf statt Fuchs (Lykos = griech. „Wolf“), Desktop-Icons neu erzeugt.
- RESYNC/Reconnect lädt den Zustand inkl. Präsenz neu.

### Behoben

- Scrollbar im Nachrichtenfeld.

## [0.2.0] – 2026-10-29

### Hinzugefügt

- Webcam in Sprachkanälen (Recht `VIDEO`, Migration 0007 setzt es auf alle Standardrollen) mit Video-Ansicht.

### Geändert

- Vektor-Icons (`lucide-react`) statt Emojis/Unicode-Symbolen.

## [0.1.0] – 2026-10-29

Erste Version (MVP 1).

### Hinzugefügt

- Konten (Benutzername + Passwort, Rate-Limit), Communities, Text- und Sprachkanäle, Rollen mit Rechten und
  Kanal-Overrides, Einladungen, Audit-Log, Echtzeit-Gateway (WebSocket).
- Voice und Bildschirmübertragung über LiveKit (Mute/Deafen, Push-to-Talk, Geräteauswahl, Sprechanzeige).
- Dateianhänge mit Bildvorschau, Größenlimit und Aufbewahrung.
- Präsenz, Tipp-Anzeige, Ungelesen-Markierung.
- Desktop-App für Windows (Tauri 2): Token im OS-Schlüsselbund, Instanzwahl, Benachrichtigungen, Installer.
- Docker-Compose-Deployment (Server, Postgres, LiveKit).
