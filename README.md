# Lykos

**Lykos** (griech. „Wolf“) ist eine datenschutzfreundliche, selbst gehostete Alternative zu Discord: Text-Chat,
Sprach- und Videoanrufe, Bildschirmübertragung und Dateien – ohne Telefonnummer, ohne Ausweis, ohne Tracking.
Server in Rust, Web- und Desktop-Client (Tauri) aus derselben React-Codebasis, Medien über einen eigenen
LiveKit-Server.

> Status: aktiv entwickelt, im Alltagseinsatz für kleine Gruppen. Version 0.x – Schnittstellen und
> Datenbankschema können sich noch ändern (Migrationen laufen automatisch).

Ziele, Grundsätze und Roadmap: [PROJECT.md](PROJECT.md) · Änderungen: [CHANGELOG.md](CHANGELOG.md) ·
API: [docs/api.md](docs/api.md)

## Funktionen

- **Communities, Kanäle, Rollen:** Text- und Sprachkanäle, fein granulare Rechte (Bitflags pro Rolle,
  Kanal-Overrides, Rollenhierarchie), Einladungen, Audit-Log, Systemkanal, Icon/Banner/Akzentfarbe.
- **Nachrichten:** Markdown (eigener Parser, kein HTML), Antworten, Reaktionen, Custom-Emotes pro Community,
  @Erwähnungen inkl. `@everyone`, angepinnte Nachrichten, Bearbeiten/Löschen, Volltextsuche, Ungelesen-Markierung.
- **Direktnachrichten:** 1:1 und Gruppen (bis 10), nur mit gemeinsamer Community, abschaltbar.
- **Voice & Video:** Sprachkanäle mit Call-Bühne, Webcam, Bildschirmübertragung, Push-to-Talk (auf dem Desktop
  global), Mute/Deafen, Lautstärke pro Teilnehmer, serverseitiger Rechteentzug im laufenden Anruf.
- **Dateien:** Anhänge mit Bildvorschau, Größenlimit und Kontingenten pro Nutzer/Community; lokal oder in S3.
- **Konto & Sicherheit:** Benutzername + Passwort, optionale E-Mail (nur für Wiederherstellung), 2FA (TOTP,
  E-Mail-Code, Wiederherstellungscodes), Anmeldung über Discord/GitHub/Google oder beliebiges OIDC, Sitzungen
  abmelden, Datenexport (JSON), Konto löschen.
- **Profile & Präsenz:** Avatar, Banner, Über-mich, Pronomen, Nicknames, Online-Status, Tipp-Anzeige.
- **Instanz-Verwaltung:** Registrierung offen/nur mit Einladung/geschlossen, Nutzer sperren, Dashboard,
  Ankündigungs-Banner, Instanz-Audit-Log, Feedback-Tool (optional als Gitea-Issue).
- **Desktop-App (Windows):** Token im OS-Schlüsselbund, Benachrichtigungen, stilles Auto-Update mit signierten
  Paketen, freie Wahl der Instanz.
- **Datenschutz:** keine Analytics, keine Tracker, keine externen Schriften/CDNs; Sounds werden lokal synthetisiert.

## Screenshots

![Chat einer Community mit Kanälen, Mitgliederliste, Antworten, Reaktionen und Erwähnungen](docs/screenshots/chat.png)

| Freunde | Ton-Einstellungen |
|---|---|
| ![Freundesliste mit Status, Nachricht, Anruf und offener Anfrage](docs/screenshots/friends.png) | ![Sprache & Video: Mikrofon mit Lautstärke und Mikrofontest, Lautsprecher mit Testton](docs/screenshots/settings-voice.png) |

| Handy | | Download |
|---|---|---|
| <img src="docs/screenshots/mobile-chat.png" alt="Chat auf dem Handy" width="220"> | <img src="docs/screenshots/mobile-drawer.png" alt="Schublade mit Communities und Kanälen auf dem Handy" width="220"> | <img src="docs/screenshots/download.png" alt="Download-Seite der Desktop-App mit Prüfsumme und Neuerungen" width="240"> |

_Demo-Daten auf einer lokalen Instanz; alle Personen sind erfunden._

## Selbst hosten

Lykos läuft als Docker-Compose-Stack aus drei Diensten auf **einem** Host:

| Dienst | Aufgabe | Ports |
|---|---|---|
| `server` | Rust-Server, liefert API (`/api`), WebSocket-Gateway (`/api/gateway`) und Web-Client | 8080/tcp (hinter Reverse Proxy) |
| `postgres` | Datenbank | intern |
| `livekit` | SFU für Voice/Video/Screenshare (Host-Netzwerk) | 7880/tcp (Signalisierung, hinter Reverse Proxy), 7881/tcp, 3478/udp, 50000–50250/udp (direkt erreichbar) |

### Voraussetzungen

- Linux-Host mit Docker und Docker Compose, öffentliche IPv4 (bzw. Portweiterleitungen), ≥ 2 GB RAM.
  Shared Hosting reicht nicht – WebRTC braucht offene UDP-Ports.
- Zwei DNS-Namen, z. B. `chat.example.org` (Lykos) und `rtc.chat.example.org` (LiveKit-Signalisierung).
- Ein Reverse Proxy mit TLS (Caddy, nginx, Traefik, Nginx Proxy Manager, …).
- Optional: SMTP-Zugang (E-Mail-Wiederherstellung, 2FA per Mail), S3-kompatibler Speicher.

### Schnellstart mit Docker Compose

```bash
git clone <repo-url> lykos && cd lykos/deploy
cp .env.example .env
# .env bearbeiten: LYKOS_PUBLIC_URL, LYKOS_LIVEKIT_URL und Secrets setzen, z. B.
#   POSTGRES_PASSWORD=$(openssl rand -hex 24)
#   LIVEKIT_API_KEY=lk$(openssl rand -hex 6)
#   LIVEKIT_API_SECRET=$(openssl rand -hex 32)
# LiveKit liest keine Umgebungsvariablen in der YAML: Webhook-Key eintragen
sed -i "s|__LIVEKIT_API_KEY__|<dein LIVEKIT_API_KEY>|" livekit.yaml
docker compose up -d --build
```

Der erste Build dauert einige Minuten (Rust + Web-Client). Danach unter `LYKOS_PUBLIC_URL` registrieren – das erste
Konto wird Instanz-Admin.

Alternativ übernimmt `deploy/deploy.sh <ssh-ziel>` das Ganze per SSH: Es überträgt den committeten Stand nach
`/opt/lykos`, legt beim ersten Lauf `/etc/lykos/lykos.env` mit generierten Secrets an, ergänzt später neue Variablen
aus `.env.example`, setzt den Webhook-Key ein und startet den Stack neu.

Datenbank-Backup: `docker compose exec postgres pg_dump -U lykos -Fc lykos > lykos-$(date +%F).dump`.
Migrationen laufen beim Serverstart automatisch.

### Reverse Proxy

Lykos und die LiveKit-Signalisierung brauchen je einen TLS-Host mit WebSocket-Unterstützung. Uploads bis
`LYKOS_MAX_UPLOAD_MB` müssen durchgelassen werden. Beispiel **Caddy**:

```caddyfile
chat.example.org {
    request_body {
        max_size 100MB
    }
    reverse_proxy 127.0.0.1:8080
}

rtc.chat.example.org {
    reverse_proxy 127.0.0.1:7880
}
```

Beispiel **nginx** (Ausschnitt, TLS-Konfiguration weggelassen):

```nginx
server {
    server_name chat.example.org;
    client_max_body_size 100m;
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 1h;
    }
}
# rtc.chat.example.org analog mit proxy_pass http://127.0.0.1:7880;
```

Hinter einem Reverse Proxy die Adresse des Proxys in `LYKOS_TRUSTED_PROXIES` eintragen (z. B. `10.0.0.5` oder
`172.16.0.0/12`): Nur Verbindungen von dort dürfen per `X-Forwarded-For` die Client-IP fürs Rate-Limit angeben.
Der Compose-Stack setzt zusätzlich `LYKOS_TRUST_PROXY=true`; ohne `LYKOS_TRUSTED_PROXIES` traut Lykos dann **jeder**
Verbindung (der Server warnt beim Start) – wer Port 8080 direkt erreicht, könnte so die IP-Limits umgehen. Ohne Proxy
`LYKOS_TRUST_PROXY=false` setzen.

### LiveKit (Voice, Video, Screenshare)

- Konfiguration: `deploy/livekit.yaml`. Medien laufen **nicht** über den Reverse Proxy, sondern direkt:
  UDP 50000–50250 (RTC 50000–50200, TURN-Relay 50201–50250), TCP 7881 (Fallback), UDP 3478 (TURN). Hinter NAT diese
  Ports auf den Host weiterleiten; `use_external_ip: true` ermittelt die öffentliche IP per STUN.
- `LYKOS_LIVEKIT_URL` ist die öffentliche Signalisierungs-URL (`wss://rtc.chat.example.org`). Fehlen URL, Key oder
  Secret, ist Voice abgeschaltet.
- LiveKit meldet Beitritte/Austritte per Webhook an `http://127.0.0.1:8080/api/livekit/webhook` (signiert mit dem
  API-Key, Platzhalter `__LIVEKIT_API_KEY__` in `livekit.yaml`).
- Zum Testen ohne eigene Infrastruktur: `docker run --network host livekit/livekit-server:v1.13.11 --dev --bind 0.0.0.0`
  (Keys `devkey`/`secret`).

### Konfiguration (Umgebungsvariablen)

| Variable | Standard | Bedeutung |
|---|---|---|
| `LYKOS_DATABASE_URL` | – (Pflicht) | Postgres-URL (im Compose-Stack automatisch) |
| `LYKOS_BIND` | `0.0.0.0:8080` | Listen-Adresse |
| `LYKOS_PUBLIC_URL` | `http://localhost:8080` | Öffentliche URL der Instanz |
| `LYKOS_SOURCE_URL` | `https://git.zeasy.dev/ZeasyAlex/Lykos` | Quellcode der laufenden Fassung, verlinkt als „Quellcode (AGPL-3.0)“. **Wer Lykos verändert betreibt, muss hier seinen eigenen Code angeben** (AGPL §13) |
| `LYKOS_INSTANCE_NAME` | `Lykos` | Anzeigename |
| `LYKOS_WEB_DIR` | – | Verzeichnis des gebauten Web-Clients (im Image gesetzt) |
| `LYKOS_CORS_ORIGINS` | Tauri-Origins | Kommagetrennte erlaubte Origins (CORS und Gateway-WebSocket; der eigene Origin aus `LYKOS_PUBLIC_URL` ist immer erlaubt) |
| `LYKOS_TRUST_PROXY` | `false` (Compose: `true`) | Client-IP fürs Login-Rate-Limit aus `X-Forwarded-For`. Ohne `LYKOS_TRUSTED_PROXIES` von jeder Verbindung – nur, wenn Lykos ausschließlich über den Proxy erreichbar ist |
| `LYKOS_TRUSTED_PROXIES` | – | IPs/CIDRs der Reverse Proxys, kommagetrennt. Gesetzt, gilt `X-Forwarded-For` nur für Verbindungen von dort (empfohlen) |
| `LYKOS_STORAGE` | `local` | Speicher für Anhänge: `local` (Verzeichnis `LYKOS_STORAGE_DIR`) oder `s3` |
| `LYKOS_STORAGE_DIR` | `./data/files` (Image: `/app/data/files`) | Verzeichnis für Dateien bei `local`; im Compose-Stack Volume `files` |
| `LYKOS_S3_ENDPOINT` | – (AWS S3) | S3-Endpunkt mit `http://`/`https://`, z. B. `http://garage:3900` |
| `LYKOS_S3_BUCKET` | – (Pflicht bei `s3`) | Bucket – Lykos allein vorbehalten (bzw. das Präfix) |
| `LYKOS_S3_REGION` | `us-east-1` | Region (Garage: meist `garage`) |
| `LYKOS_S3_ACCESS_KEY_ID` / `LYKOS_S3_SECRET_ACCESS_KEY` | – (Pflicht bei `s3`) | Zugangsschlüssel |
| `LYKOS_S3_PATH_STYLE` | `true` | `true` = `{endpoint}/{bucket}/{key}` (Garage, MinIO); `false` = virtuelle Hosts (AWS) |
| `LYKOS_S3_PREFIX` | – | Optionales Schlüssel-Präfix im Bucket |
| `LYKOS_MAX_UPLOAD_MB` | `100` | Maximale Upload-Größe in MB (Reverse Proxy muss mindestens so viel durchlassen) |
| `LYKOS_USER_QUOTA_MB` | `2048` | Kontingent pro Nutzer für Anhänge in MB (`0` = unbegrenzt); pro Community zusätzlich einstellbar |
| `LYKOS_LIVEKIT_URL` | – | Öffentliche LiveKit-URL für Clients (`wss://…`). Fehlt sie oder Key/Secret, ist Voice aus |
| `LIVEKIT_API_KEY` / `LIVEKIT_API_SECRET` | – | LiveKit-Zugangsdaten (Tokens, Webhook-Prüfung, RoomService) |
| `LYKOS_LIVEKIT_API_URL` | `http://127.0.0.1:7880` | Interne HTTP-URL der LiveKit-API; im Compose-Stack `http://host.docker.internal:7880` |
| `LYKOS_DESKTOP_RELEASES_URL` | – | Gitea-API einer Release-Liste für `/download` und das Update-Manifest der Desktop-App (`.env.example`: offizielle Releases). Leer = aus |
| `LYKOS_FEEDBACK_GITEA_URL` / `_TOKEN` | – | Optional: Feedback-Meldungen per Admin-Klick als Gitea-Issue anlegen |
| `LYKOS_MAIL` | automatisch | `smtp`, `log` (nur Entwicklung: Mails ins Log) oder `off`. Ohne Angabe `smtp`, sobald SMTP konfiguriert ist |
| `LYKOS_MAIL_FROM` | – (Pflicht bei SMTP) | Absenderadresse; angezeigt als „Lykos“ |
| `LYKOS_SMTP_HOST` / `_PORT` / `_USER` / `_PASSWORD` | – | SMTP-Zugang (Port leer = Standard der TLS-Art) |
| `LYKOS_SMTP_TLS` | `implicit` | `implicit` (meist 465), `starttls` (meist 587) oder `none` (nur lokale Relays) |
| `LYKOS_SMTP_URL` | – | Alternative: `smtps://user:pass@host:465` bzw. `smtp://user:pass@host:587`; Sonderzeichen URL-kodieren |
| `LYKOS_OAUTH_PROVIDERS` | – | Externe Anmeldung, z. B. `discord,github,google` |
| `LYKOS_OAUTH_<NAME>_CLIENT_ID` / `_CLIENT_SECRET` | – | Zugangsdaten der App beim Anbieter |
| `LYKOS_OAUTH_<NAME>_LABEL` / `_SCOPES` | Preset bzw. Name / Preset | Button-Beschriftung, Scopes (Leerzeichen-getrennt) |
| `LYKOS_OAUTH_<NAME>_ISSUER` | – | Nur generisches OIDC: Issuer-URL (Discovery unter `/.well-known/openid-configuration`) |
| `RUST_LOG` | `lykos_server=info,tower_http=info` | Log-Filter |

Im Compose-Stack werden die Variablen in `deploy/docker-compose.yml` an den Server durchgereicht; neue (z. B. für
einen generischen OIDC-Anbieter) dort ergänzen.

### E-Mail (optional)

E-Mail ist in Lykos **freiwillig** und dient nur der Kontowiederherstellung und optional als zweiter Faktor. Ohne
SMTP meldet `/api/instance` `email: false`, und der Client blendet E-Mail-Feld, „Passwort vergessen?“ und 2FA per
E-Mail aus.

```env
LYKOS_MAIL_FROM=lykos@example.org
LYKOS_SMTP_HOST=smtp.example.org
LYKOS_SMTP_PORT=465
LYKOS_SMTP_TLS=implicit        # oder: 587 + starttls
LYKOS_SMTP_USER=lykos@example.org
LYKOS_SMTP_PASSWORD=geheim!mit:sonderzeichen
```

Einzelvariablen brauchen kein URL-Encoding. Beim Start loggt der Server `Mailversand: SMTP` (ohne Passwort).
SPF/DKIM für die Absenderdomain einrichten, sonst landen Mails im Spam. Mails enthalten keine externen Bilder und
kein Tracking; Links zeigen auf `LYKOS_PUBLIC_URL/verify-email#token=…` bzw. `/reset-password#token=…` (Token im
Fragment – nicht in Server-/Proxy-Logs oder im `Referer`).

### Anmeldung mit Discord, GitHub, Google oder OIDC (optional)

Die Login-Seite zeigt dann „Mit … anmelden“. Lykos übernimmt vom Anbieter **nur die Konto-ID** – keinen Namen, kein
Bild; eine bestätigte E-Mail-Adresse nur, wenn der Nutzer zustimmt. Neue Konten entstehen nach denselben Regeln wie
sonst (Benutzername wählen, Einladung bei `invite_only`). In der Desktop-App sind die Buttons vorerst ausgeblendet.

Redirect-URI ist immer `<LYKOS_PUBLIC_URL>/api/auth/oidc/<name>/callback`, z. B.
`https://chat.example.org/api/auth/oidc/discord/callback`.

- **Discord** (Preset `discord`): <https://discord.com/developers/applications> → *New Application* → *OAuth2* →
  Redirect eintragen → Client-ID/Secret in `LYKOS_OAUTH_DISCORD_CLIENT_ID`/`_CLIENT_SECRET`. Standard-Scope
  `identify`; mit `LYKOS_OAUTH_DISCORD_SCOPES=identify email` kann die bestätigte Adresse übernommen werden.
- **GitHub** (Preset `github`): *Settings* → *Developer settings* → *OAuth Apps* → *New OAuth App*, *Homepage URL* =
  `LYKOS_PUBLIC_URL`, Callback wie oben → `LYKOS_OAUTH_GITHUB_CLIENT_ID`/`_CLIENT_SECRET`. Ohne Scope.
- **Google** (Preset `google`): <https://console.cloud.google.com> → *OAuth-Zustimmungsbildschirm* (Extern, Scopes
  `openid`, `email`) → *OAuth-Client-ID* (Webanwendung, Weiterleitungs-URI wie oben) →
  `LYKOS_OAUTH_GOOGLE_CLIENT_ID`/`_CLIENT_SECRET`. Für alle Nutzer den Bildschirm auf *In production* stellen
  (braucht eine Datenschutzerklärungs-URL).
- **Generisches OIDC** (Keycloak, Authentik, Zitadel, …): Client vom Typ *confidential* mit Authorization Code +
  PKCE, dann z. B. `LYKOS_OAUTH_PROVIDERS=firma`, `LYKOS_OAUTH_FIRMA_ISSUER=https://sso.example.org/realms/lykos`,
  `LYKOS_OAUTH_FIRMA_CLIENT_ID`, `LYKOS_OAUTH_FIRMA_CLIENT_SECRET`, `LYKOS_OAUTH_FIRMA_LABEL=Firmen-Login`. Reines
  OAuth2 ohne OIDC über `_AUTHORIZE_URL`, `_TOKEN_URL`, `_USERINFO_URL` und `_SUBJECT_FIELD` statt `_ISSUER`.

Sicherheit: `state` (an den Browser gebunden per kurzlebigem Cookie), PKCE (S256), bei OIDC Prüfung von Signatur
(nur asymmetrisch, JWKS), `iss`, `aud`/`azp`, `exp` und `nonce`; bei OAuth2 Identität über den Userinfo-Endpunkt.

### Dateispeicher in S3 (optional)

Mit `LYKOS_STORAGE=s3` landen Anhänge in einem S3-kompatiblen Objektspeicher (Garage, MinIO, AWS). Uploads werden
gestreamt (ab 8 MiB als Multipart-Upload), Downloads mit `Range` gehen als Range-Request an S3.

- **Bucket exklusiv:** Der Aufräum-Job löscht im Bucket (bzw. unter `LYKOS_S3_PREFIX`) Objekte im
  Lykos-Schlüsselformat ohne zugehörigen Anhang (älter als 1 h).
- **Abgebrochene Multipart-Uploads:** Lifecycle-Regel `AbortIncompleteMultipartUpload` (z. B. nach 1 Tag) setzen.
- **Umzug lokal → S3:** Lokal liegen Dateien unter `<dir>/<letzte 2 Zeichen>/<id>`, in S3 direkt unter
  `<präfix>/<id>`. Alle Dateien flach in den Bucket kopieren, z. B.
  `find data/files -type f -not -path '*/.tmp/*' -exec aws s3 cp {} s3://<bucket>/<präfix>/ \;`.

### Desktop-App für die eigene Instanz

Die offizielle Desktop-App fragt beim ersten Start nach der Server-Adresse (Standard ist die Instanz des
Herausgebers) und kann mit jeder Lykos-Instanz verbunden werden, deren `LYKOS_CORS_ORIGINS` die Tauri-Origins
erlaubt (Standard). Die Download-Seite `/download` der eigenen Instanz zeigt das neueste Release aus
`LYKOS_DESKTOP_RELEASES_URL`.

## Entwicklung

Voraussetzungen: Rust (stable), Node 24, Docker.

```bash
# Postgres starten (nur die DB aus dem Compose-Stack; Port 5432 z. B. per docker-compose.override.yml freigeben)
cd deploy && cp .env.example .env   # POSTGRES_PASSWORD setzen
docker compose up -d postgres

# Server (Port 8080)
LYKOS_DATABASE_URL=postgres://lykos:<passwort>@localhost:5432/lykos cargo run -p lykos-server

# Web-Client (Port 5173, /api wird auf :8080 durchgereicht)
cd client && npm install && npm run dev

# Desktop-Client gegen den lokalen Server
cd client && VITE_LYKOS_SERVER=http://localhost:8080 npm run tauri dev
```

Tests und Prüfungen (wie in der CI):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
DATABASE_URL=postgres://lykos:<passwort>@localhost:5432/lykos cargo test --workspace --locked
cd client && npm run lint && npm test && npm run build
```

S3-Tests (`cargo test --test s3`) laufen nur, wenn `LYKOS_TEST_S3_BUCKET` (plus `…_ENDPOINT`, `…_ACCESS_KEY_ID`,
`…_SECRET_ACCESS_KEY`) gesetzt ist. `LYKOS_MAIL=log` schreibt Mails samt Codes ins Server-Log.

### Projektstruktur

| Pfad | Inhalt |
|---|---|
| `crates/lykos-server` | Rust-Backend (axum, sqlx/Postgres). Liefert API unter `/api` und den Web-Client aus |
| `crates/lykos-server/migrations` | SQL-Migrationen, laufen beim Serverstart automatisch |
| `client/` | React + TypeScript (Vite) – Web-Client und Oberfläche der Desktop-App |
| `client/src-tauri` | Tauri-2-Hülle für den Desktop-Client (eigenes Cargo-Projekt) |
| `deploy/` | Docker Compose (Server, Postgres, LiveKit), LiveKit-Config, Deploy-Skript |
| `docs/api.md` | REST-API und Gateway-Events |
| `Dockerfile` | Ein Image: baut Web-Client und Server, liefert beides aus |

## Desktop-App bauen (Windows)

Voraussetzungen zusätzlich: Rust (stable, MSVC), WebView2 (ab Windows 10 vorhanden). Den NSIS-Installer lädt die
Tauri-CLI beim ersten Build selbst herunter.

```powershell
pwsh scripts/release-desktop.ps1            # npm ci + tauri build (NSIS) + signierte Update-Artefakte
pwsh scripts/release-desktop.ps1 -SkipInstall
```

Ergebnis in `dist-desktop/`: `Lykos-<version>-windows-x64-setup.exe` mit `.sha256` und `.sig` (Updater-Signatur)
sowie ein statisches `latest.json`. Die Version kommt aus `client/src-tauri/tauri.conf.json`.

**Eigene Builds / Forks:** `createUpdaterArtifacts` signiert mit einem privaten Tauri-Updater-Schlüssel
(Standard `%USERPROFILE%\.tauri\lykos-updater.key` + `.password`, nie ins Repo). Für eigene Builds:

1. Schlüssel erzeugen: `npx tauri signer generate -w "$env:USERPROFILE\.tauri\lykos-updater.key"`.
2. In `client/src-tauri/tauri.conf.json` `plugins.updater.pubkey` durch den eigenen Public Key und
   `plugins.updater.endpoints` durch `https://<deine-instanz>/api/desktop/latest.json` ersetzen; bei einem
   eigenständigen Fork auch `identifier`, `productName` und `DEFAULT_SERVER_URL` in `client/src/lib/api.ts`.
3. Release mit `.exe`, `.exe.sha256`, `.exe.sig` in einem Gitea-Repo veröffentlichen und
   `LYKOS_DESKTOP_RELEASES_URL` darauf zeigen lassen.

Ohne Updater-Signatur: `npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'`.

Besonderheiten des Desktop-Bundles:

- **Instanzwahl** beim ersten Start und im Anmeldebildschirm („Ändern“); lokal gespeichert.
- **Session-Token im OS-Schlüsselbund** (Windows-Anmeldeinformationsverwaltung, macOS-Schlüsselbund, Linux Secret
  Service), ein Eintrag pro Instanz, nicht im `localStorage`.
- **Benachrichtigungen** bei unfokussiertem Fenster (außer „Bitte nicht stören“ und stummgeschaltete Kanäle).
- **Globales Push-to-Talk** (`tauri-plugin-global-shortcut`); unter Windows bekommen andere Programme die Taste
  währenddessen nicht – am besten eine sonst ungenutzte Taste (F13–F24, Pause, Rollen).
- **Auto-Update:** Beim Start Splash „Suche nach Updates …“ (max. 4 s), stille Installation (NSIS `quiet`, keine
  UAC-Abfrage dank `currentUser`); im laufenden Betrieb alle 6 h Hintergrund-Download und grüner Pfeil „Neustart
  zum Aktualisieren“ (im Sprachanruf gesperrt). Nur Pakete mit gültiger Signatur. Logik:
  `client/src/lib/updater.ts`, `client/src/lib/updateFlow.ts`.
- **Mikrofon/Kamera ohne Rückfrage (Windows):** WebView2-Berechtigungsanfragen der eigenen Herkunft
  (`http://tauri.localhost`) werden selbst erlaubt (`client/src-tauri/src/permissions.rs`); andere Herkünfte
  bekommen Mikrofon/Kamera nie. Der Windows-Datenschutzschalter gilt weiterhin.
- **CSP:** `connect-src https: wss:` (Instanz frei wählbar), Bilder/Medien aus `https:`, `data:`/`blob:`.

## Mitmachen

Issues und Pull Requests auf **GitHub: <https://github.com/ZeasySoftware/Lykos>** (Spiegel des Originals auf
<https://git.zeasy.dev/ZeasyAlex/Lykos>). Beiträge sind willkommen – siehe [CONTRIBUTING.md](CONTRIBUTING.md) und den
[Verhaltenskodex](CODE_OF_CONDUCT.md). Sicherheitslücken bitte **nicht** öffentlich melden, sondern wie in
[SECURITY.md](SECURITY.md) beschrieben.

## Lizenz

Copyright © 2026 Zeasy Software.

Lykos steht unter der **GNU Affero General Public License v3.0 oder später** (`AGPL-3.0-or-later`), siehe
[LICENSE](LICENSE) und [NOTICE](NOTICE). Wer eine veränderte Fassung als Netzwerkdienst betreibt, muss deren
Quellcode den Nutzern anbieten (AGPL §13). Drittkomponenten: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
