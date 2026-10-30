# Sicherheitsrichtlinie

## Unterstützte Versionen

Lykos ist in der 0.x-Phase. Sicherheitskorrekturen gibt es nur für die **jeweils neueste veröffentlichte
Version**; Betreiber sollten zeitnah aktualisieren (Server: neues Image bauen/deployen; Desktop-App: aktualisiert
sich selbst).

| Version | Unterstützt |
|---|---|
| neueste 0.x | ja |
| ältere | nein |

## Sicherheitslücke melden

Bitte **keine öffentlichen Issues, Pull Requests oder Chat-Nachrichten** für Sicherheitslücken.

Stattdessen eine E-Mail an **info@zeasy.software** mit:

- betroffener Komponente (Server, Web-Client, Desktop-App, Deployment) und Version/Commit,
- Beschreibung und Auswirkung,
- Schritten zum Nachvollziehen bzw. Proof of Concept,
- optional einem Vorschlag zur Behebung und wie du genannt werden möchtest.

Wenn die Plattform es anbietet, ist auch eine private Sicherheitsmeldung über das Repository („Report a
vulnerability“ / Security Advisory) möglich.

## Ablauf

1. Eingangsbestätigung innerhalb von **5 Werktagen**.
2. Erste Einschätzung (Schweregrad, betroffene Versionen) innerhalb von **14 Tagen**.
3. Behebung und Veröffentlichung einer korrigierten Version; bei kritischen Lücken so schnell wie möglich.
4. Koordinierte Offenlegung: Wir veröffentlichen einen Hinweis (CHANGELOG/Advisory), sobald eine Korrektur
   verfügbar ist, spätestens nach **90 Tagen** in Absprache mit dir. Auf Wunsch nennen wir dich als Finder.

Es gibt derzeit kein Bug-Bounty-Programm.

## Im Rahmen

- Server (`crates/lykos-server`): Authentifizierung, 2FA, OAuth/OIDC, Rechteprüfung, Gateway, Uploads, S3,
  LiveKit-Webhooks, Rate-Limits.
- Web- und Desktop-Client (`client/`, `client/src-tauri`): XSS, CSP-Umgehung, Token-Handhabung,
  Updater-Signaturprüfung, WebView2-Berechtigungen.
- Die mitgelieferte Deployment-Konfiguration (`deploy/`, `Dockerfile`).

## Nicht im Rahmen

- Fehlkonfiguration einer einzelnen selbst gehosteten Instanz (bitte an deren Betreiber wenden).
- Lücken in Drittprojekten (LiveKit, PostgreSQL, Tauri, …) – bitte dort melden; Hinweise auf betroffene Lykos-
  Versionen nehmen wir gerne entgegen.
- Denial of Service durch reine Last, Social Engineering, physischer Zugriff.
- Fehlende Code-Signatur des Windows-Installers (bekannt, SmartScreen-Warnung).
