# Changelog

Alle nennenswerten Änderungen an Lykos stehen in dieser Datei.

Das Format orientiert sich an [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), die Versionierung an
[Semantic Versioning](https://semver.org/lang/de/) (in der 0.x-Phase: Minor = neue Funktionen bzw. Migrationen,
Patch = Fehlerbehebungen). Migrationen laufen beim Serverstart automatisch.

## [Unveröffentlicht]

## [0.6.0] – 2026-10-30

### Hinzugefügt

- **Direktnachrichten anpinnen:** Unterhaltungen und Gruppen lassen sich für sich selbst anpinnen (bis 20) und
  stehen dann im Abschnitt „Angepinnt“ ganz oben – auf allen eigenen Geräten gleich. Über das Kontextmenü oder das
  Pin-Symbol am Eintrag.
- **Personen hinzufügen:** Im Kopf einer DM, im Kontextmenü und unter „Gruppe bearbeiten“. In einer Gruppe kommen
  die Ausgewählten dazu (mit Hinweis im Verlauf, ein laufender Anruf geht weiter), aus einer 1:1-DM entsteht eine
  neue Gruppe – die Unterhaltung zu zweit bleibt.
- **Gruppe erstellen:** Eigener Knopf in der DM-Seitenleiste mit Gruppenname und Personenauswahl; eine Gruppe geht
  auch mit nur einer weiteren Person oder erst einmal allein (`POST /dms` mit `group: true`).
- **AutoMod** pro Community (Einstellungen → „AutoMod“, `MANAGE_COMMUNITY`): Wortfilter mit Platzhaltern und Ausnahmen,
  Links nur zu erlaubten Domains, Erwähnungs-Spam und Flut/Wiederholungen. Je Regel blockieren, Alarm in einen
  Protokollkanal und/oder Audit-Log; Ausnahmen für Rollen und Kanäle, Verwalter sind ausgenommen. Prüft auch
  bearbeitete Nachrichten – auch von Bots (wie Mitglieder, mit ihren Rollen) und Webhooks.
- **Bannen:** im Mitglieder-Kontextmenü, auf der Profilkarte und in der Mitgliederliste der Einstellungen, mit
  optionalem Grund und „Nachrichten der letzten Stunde … 7 Tage löschen“. Gebannte fliegen sofort raus und kommen
  über keine Einladung zurück; neuer Tab „Banns“ mit Suche und „Bann aufheben“. Neues Recht „Mitglieder bannen“
  (`BAN_MEMBERS`) – Rollen, die bisher Mitglieder entfernen durften, bekommen es automatisch.
- **AFK-Kanal:** In der Übersicht der Community-Einstellungen wählbar (Sprachkanal + 1/5/15/30/60 Minuten). Wer so lange
  weder spricht noch die App bedient, wechselt automatisch dorthin (nicht beim Streamen oder mit Kamera); im
  AFK-Kanal kann niemand sprechen, streamen oder die Kamera nutzen. Markiert mit einem Mond in der Kanalleiste.
- **Willkommensnachricht:** Beitrittsmeldungen abschaltbar, optional eigener Text mit `{user}` und `{community}` (Markdown,
  Vorschau in „Darstellung“).
- **Kanal-Kategorien** wie bei Discord: einklappbare Gruppen in der Seitenleiste, „Kategorie erstellen“ im
  Community-Menü, umbenennen/löschen/„Kanal hier erstellen“ an der Kategorie, Kategorie-Auswahl beim Anlegen und in
  den Kanal-Einstellungen. Bestehende Communities sehen unverändert aus.
- **Import aus Discord-Servervorlagen** beim Anlegen einer Community (`discord.new/…`-Link): Vorschau, dann werden Rollen,
  Kategorien, Kanäle, Kanal-Rechte, Systemkanal, AFK-Kanal und Beschreibung übernommen. Nachrichten, Mitglieder, Emotes und Bots
  nicht. Der Server ruft dafür die öffentliche Discord-API ab (nur den Vorlagen-Code, ohne Nutzerdaten).
- **Eingehende Webhooks** pro Textkanal (neues Recht „Webhooks verwalten“, `MANAGE_WEBHOOKS = 1 << 18`): anlegen,
  umbenennen, Avatar, Kanal wechseln, Token neu erzeugen, löschen – mit Audit-Log. Ausführen ohne Anmeldung unter
  `POST /api/webhooks/{id}/{token}`, kompatibel mit Discord-Webhooks (Gitea/Forgejo, Grafana, Uptime Kuma, Skripte):
  `content`, `username`, Embeds als Karten, `allowed_mentions` (Standard: kein `@everyone`), `?wait=true`,
  Bearbeiten/Löschen eigener Nachrichten, 30 Aufrufe pro Minute. Aus Datenschutzgründen lädt Lykos keine fremden
  Bilder: `avatar_url` und Bilder in Embeds werden ignoriert. Dateien per Webhook gehen (noch) nicht.
- **Bot-Konten:** Unter Einstellungen → „Entwickler“ eigene Bots anlegen (bis 10), Token anzeigen/neu erzeugen,
  löschen; Anmeldung mit `Authorization: Bot <token>` für REST und Gateway, Konto-/Freundes-/DM-Endpunkte sind für
  Bots gesperrt. Bots per ID zur Community hinzufügen (eigene oder öffentliche), eigene Gruppe „Bots“ in der
  Mitgliederliste, BOT-Kennzeichen an Namen. Instanz-Admins können Bots abschalten. Anleitung: `docs/bots.md`.

### Geändert

- Neue Communities nutzen ihren ersten Textkanal als Systemkanal für Beitrittsmeldungen.
- Der Kopf der DM-Seitenleiste heißt „Nachrichten“.

### Behoben

- Desktop: Dateien lassen sich wieder per Drag-and-drop in die App ziehen (Tauri fing Drops selbst ab).

### Betrieb

- Migrationen **0022–0026**: DM-Pins (`dm_recipients.pinned_at`, Nachrichtenart `recipient_add`), Kanal-Kategorien
  (`channel_categories`, `channels.category_id`), Banns/AFK/Willkommen (`community_bans`, neue Community-Spalten,
  Recht `BAN_MEMBERS = 1 << 17`), AutoMod (`automod_rules`, Nachrichtenart `automod_alert`), Webhooks/Bots
  (`webhooks`, `bots`, neue Spalten in `users` und `messages`, Recht `MANAGE_WEBHOOKS = 1 << 18`). Vor dem Update
  ein Datenbank-Backup anlegen.
- Der Discord-Import ruft `https://discord.com/api/v10/guilds/templates/{code}` vom Server aus ab (ausgehende
  HTTPS-Verbindung zu discord.com nötig).

## [0.5.1] – 2026-10-30

### Hinzugefügt

- Impressum und Datenschutzerklärung pro Instanz (`LYKOS_IMPRINT_FILE`, `LYKOS_PRIVACY_FILE`), als Seiten
  `/impressum` und `/datenschutz` verlinkt in Anmeldung, Einstellungen und Download-Seite.
- Desktop: Beim ersten Schließen fragt die App, ob sie beendet oder in den Infobereich minimiert werden soll
  (mit „Entscheidung merken“); „Nach Updates suchen“ mit Status in den Einstellungen unter „Desktop-App“.
- Rückmeldung beim Senden und Annehmen von Freundschaftsanfragen.
- Feedback-Formular verlinkt „Issue auf GitHub anlegen“.

### Geändert

- Schmale Scrollleisten, die nur beim Scrollen erscheinen.
- Release-Notizen auf der Download-Seite als Markdown; Desktop-Releases kommen von GitHub
  (`LYKOS_DESKTOP_RELEASES_URL` kann auf die GitHub-API zeigen), Quellcode-Link auf GitHub.
- Container-Protokolle begrenzt (3 × 10 MB), LiveKit protokolliert nur Warnungen.

## [0.5.0] – 2026-10-30

Erste öffentliche Version des Quellcodes. Migrationen **0018–0021** (laufen beim Start automatisch).

### Hinzugefügt

- **Freundesystem:** Anfragen per Benutzername, Profilkarte oder Kontextmenü; annehmen, ablehnen, zurückziehen,
  entfernen; **Blockieren** (keine neuen DMs, bestehende 1:1-DM nur lesbar, Nachrichten eingeklappt). Freundesliste
  im DM-Bereich (Online/Alle/Ausstehend/Blockiert), Einstellung „Wer darf mir Anfragen schicken“. DMs unter Freunden
  auch ohne gemeinsame Community.
- **Anrufe in DMs** (1:1 und Gruppe): Anruf- und Video-Knopf, Klingeln mit Annehmen/Ablehnen, Klingel- und Freiton,
  Desktop-Benachrichtigung, „Anruf verpasst“.
- **Ton-Einstellungen:** pro Sprachkanal Bitrate (32–256 kbit/s), Stereo, DTX, RED und Nutzerlimit; Mikrofon-
  Eingangslautstärke bis 200 %, Pegelanzeige, Mikrofontest, Schwelle für Sprachaktivierung, Push-to-Talk-
  Loslassverzögerung; Ausgabelautstärke, Testton, Lautstärke pro Teilnehmer bis 200 %, eigenes Gerät für
  Benachrichtigungstöne.
- **Mobil-Layout** für schmale Fenster (≤ 768 px) und installierbare Web-App (PWA-Manifest).
- **Desktop:** Schließen in den Infobereich (abschaltbar), Tray-Menü, nur eine laufende Instanz, „Mit Windows
  starten“, eigene Titelleiste.
- **HTML-Mails** im Lykos-Design (immer mit Textteil, ohne externe Bilder/Tracker); Hinweis an die alte Adresse,
  wenn die E-Mail-Adresse geändert wird.
- Anbieter-Logos an den Anmelde-Buttons, Knopf „Desktop-App installieren“ im Web, Link „Quellcode (AGPL-3.0)“
  (`LYKOS_SOURCE_URL`).
- Lizenz **AGPL-3.0-or-later**, `NOTICE`, `THIRD_PARTY_NOTICES.md`, `CONTRIBUTING.md` (DCO), `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, Issue- und PR-Vorlagen, CI-Workflow für GitHub-Forks.

### Sicherheit

- Content-Security-Policy, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` (und HSTS bei https) für den
  Web-Client; Gateway nimmt nur Verbindungen der eigenen bzw. erlaubten Origins an.
- Profile sind nur noch für Freunde und Absender einer an dich gerichteten Anfrage sichtbar (nicht mehr nach eigenem
  Blockieren oder einer eigenen Anfrage).
- Anmeldedienst verknüpfen verlangt erneute Anmeldung; Rauswerfen beachtet die Rollenhierarchie; Bearbeiten und
  Anpinnen nur mit Schreibrecht.
- Login-Sperre pro Benutzername **und** IP (kein gezieltes Aussperren fremder Konten); Limits für
  Freundschaftsanfragen, neue DMs und Nachrichten.
- `LYKOS_TRUSTED_PROXIES`: `X-Forwarded-For` gilt nur noch von den genannten Reverse Proxys.
- Der eigene Web-Client bekommt das Session-Token nicht mehr im Login-Body (nur HttpOnly-Cookie).
- Links in Mails tragen Tokens im Fragment (`#token=…`), nicht mehr in der URL-Abfrage; „Passwort vergessen“ ohne
  Zeitunterschied zwischen bekannten und unbekannten Adressen.
- Desktop: Zwischenablage wird nicht mehr automatisch lesbar, SmartScreen wieder aktiv.

### Geändert

- Deutlich schneller bei großen Instanzen: Erwähnungszählung ohne Historien-Scan, neue Indizes, Gateway-Events
  einmal serialisiert, Präsenz beim Verbinden nur für nicht-offline Nutzer, Kompression (gzip/brotli) und
  Cache-Header, kleinerer Start-Bundle durch Code-Splitting.
- README für Selbst-Hoster neu gegliedert; Gitea-CI wählt den Runner über `LYKOS_RUNNER`; `deploy/deploy.sh` ohne
  fest eingetragenes Ziel.

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
