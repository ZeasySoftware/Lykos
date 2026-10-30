# Mitmachen bei Lykos

Danke für dein Interesse! Fehlerberichte, Ideen, Übersetzungen, Doku und Code sind willkommen. Mit deiner Teilnahme
akzeptierst du den [Verhaltenskodex](CODE_OF_CONDUCT.md). Sicherheitslücken bitte **nicht** als Issue, sondern wie
in [SECURITY.md](SECURITY.md) beschrieben melden.

Projektsprache ist Deutsch (Oberfläche, Code-Kommentare, Commits, Doku). Issues und Pull Requests auf Englisch sind
ebenfalls in Ordnung.

## Bevor du loslegst

- **Fehler:** Erst nach bestehenden Issues suchen, dann die Vorlage „Fehler melden“ nutzen (Version, Plattform,
  Schritte zum Nachvollziehen).
- **Größere Änderungen** (neue Funktionen, Datenmodell, API, Abhängigkeiten): bitte zuerst ein Issue eröffnen und
  den Ansatz abstimmen. Lykos hat feste Grundsätze (siehe [PROJECT.md](PROJECT.md)) – z. B. keine Tracker, keine
  externen CDNs, minimale Datenerhebung, Icons nur über `lucide-react`, Styles nur über Design-Tokens.
- **Kleine Korrekturen** (Tippfehler, offensichtliche Bugs): direkt als Pull Request.

## Entwicklungsumgebung

Siehe [README → Entwicklung](README.md#entwicklung). Kurz: Rust (stable), Node 24, Docker für Postgres.

## Richtlinien

### Code

- **Rust:** `cargo fmt` (Konfiguration in `rustfmt.toml`), `cargo clippy --workspace --all-targets -- -D warnings`
  ohne Warnungen.
- **Client:** TypeScript strikt, `npm run lint` (oxlint) sauber. Farben, Abstände, Radien und Schatten nur über
  Tokens in `client/src/styles/tokens.css`; Styles nach Bereichen in `client/src/styles/*.css`, kein CSS-Framework.
- **Datenbank:** Schemaänderungen nur als neue Migration in `crates/lykos-server/migrations/` (bestehende
  Migrationen nie ändern).
- **API:** Neue oder geänderte Endpunkte und Gateway-Events in `docs/api.md` dokumentieren.
- **Datenschutz:** Keine neuen Daten erheben, ohne es im PR zu begründen; nichts an Dritte senden.

### Tests

- Server: Integrationstests unter `crates/lykos-server/tests/` (brauchen `DATABASE_URL` auf eine Wegwerf-Datenbank).
- Client: reine Logik in `*.test.ts` neben dem Modul (`npm test`, Node-Testrunner).
- Neue Funktionen und Bugfixes bitte mit Test.

### Commits

Kurze, aussagekräftige Nachrichten im Stil von [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(voice): Lautstärke pro Teilnehmer bis 200 %
fix(client): Banner überdeckte Icon im Community-Dropdown
docs: Selbst-Hosting mit Caddy
chore: Abhängigkeiten aktualisiert
```

Typen: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `ci`. Ein Commit = eine logische Änderung.

### Pull Requests

1. Fork anlegen, Branch von `main` (z. B. `feat/freundesliste-sortierung`).
2. Änderungen mit Tests; `CHANGELOG.md` unter „Unveröffentlicht“ ergänzen, wenn Nutzer oder Betreiber etwas davon
   merken.
3. Lokal alle Prüfungen laufen lassen (fmt, clippy, Tests, Lint, Build). Im öffentlichen Repo auf git.zeasy.dev läuft
   bewusst **keine** CI – der eigene Build-Runner führt keinen fremden Code aus. In Forks auf GitHub/Gitea laufen die
   Workflows (siehe unten).
4. PR-Vorlage ausfüllen; bei UI-Änderungen Screenshots anhängen.

So landet ein Beitrag im Code: Entwickelt wird in einem internen Repo (mit Betriebsnotizen, die nicht öffentlich
sind); das öffentliche Repo bekommt je Veröffentlichung einen Commit. Angenommene PRs übernehmen die Maintainer dort
als Patch mit deinem Sign-off, die CI prüft sie, und sie erscheinen mit der nächsten Veröffentlichung – der PR wird
dann mit Verweis darauf geschlossen und du im `CHANGELOG.md` genannt.

### Lizenz deiner Beiträge

Lykos steht unter der **AGPL-3.0-or-later**. Mit einem Pull Request erklärst du, dass du den Beitrag unter dieser
Lizenz einbringen darfst und einbringst („inbound = outbound“). Bitte bestätige das mit einem
[Developer Certificate of Origin](https://developercertificate.org/)-Sign-off in jedem Commit
(`git commit -s` → `Signed-off-by: Name <mail>`).

## CI in Forks

Die Workflows laufen ohne Anpassung in Forks:

- **GitHub:** `.github/workflows/ci.yml` auf `ubuntu-latest`.
- **Gitea/Forgejo:** `.gitea/workflows/ci.yml`. Der Runner wird über die Actions-Variable `LYKOS_RUNNER` gewählt,
  ohne Variable gilt `ubuntu-latest`. Wer einen eigenen Runner mit anderem Label hat, setzt im Repo unter
  *Einstellungen → Actions → Variablen* z. B. `LYKOS_RUNNER=mein-label`.

Die Jobs brauchen Docker (Postgres-Service-Container).

## Für Maintainer: Release-Prozess und Branch-Schutz (Vorschlag)

### Branch-Schutz für `main`

- Kein direkter Push; Änderungen nur über Pull Requests.
- Pflicht-Statuschecks: „Server (fmt, clippy, test)“ und „Client (lint, build)“.
- Mindestens eine Freigabe durch einen Maintainer; veraltete Freigaben bei neuen Commits verwerfen.
- Branch muss vor dem Merge aktuell sein; Merge per Squash oder Rebase (lineare Historie).
- Kein Force-Push, kein Löschen von `main`; Tags `v*` schützen.

### Release

1. Versionsnummer nach [SemVer](https://semver.org/) (0.x: Minor = neue Funktionen/Migrationen, Patch = Fehler)
   in allen vier Dateien anheben: `Cargo.toml` (Workspace), `client/package.json` (+ Kopf von
   `package-lock.json`), `client/src-tauri/tauri.conf.json`, `client/src-tauri/Cargo.toml`; Lockfiles mit
   `cargo update -w --offline` bzw. in `client/src-tauri` `cargo update -p lykos-client --offline`.
2. `CHANGELOG.md`: „Unveröffentlicht“ in `[x.y.z] – Datum` umbenennen.
3. Commit `chore: Version x.y.z`, PR, Merge, Tag `vx.y.z` auf den Merge-Commit.
4. Desktop: `pwsh scripts/release-desktop.ps1` mit dem Release-Signaturschlüssel, Release `vx.y.z` mit `.exe`,
   `.exe.sha256`, `.exe.sig` und dem CHANGELOG-Abschnitt als Text veröffentlichen.
5. Server: Instanzen deployen (DB-Backup vorher).

Später sinnvoll: Release-Workflow, der bei Tag `v*` Docker-Image und Installer baut, sowie
`cargo deny`/`npm audit` als zusätzliche CI-Jobs.
