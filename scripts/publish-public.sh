#!/usr/bin/env bash
# Erzeugt einen frischen öffentlichen Export von Lykos: neues Verzeichnis, Inhalt aus `git archive` ohne interne
# Dateien, neue Git-Historie mit genau einem Initial-Commit. Die bestehende Historie bleibt unangetastet; es wird
# nichts gepusht und kein Remote angelegt.
#
# Nutzung:
#   scripts/publish-public.sh --dry-run [--ref <rev>]          # nur anzeigen und prüfen, nichts schreiben
#   scripts/publish-public.sh [--ref <rev>] [--force] <ziel>   # Export nach <ziel> (darf nicht existieren/muss leer sein)
#   scripts/publish-public.sh [--ref <rev>] --into <klon>      # Update: Stand als neuen Commit in einen Klon des
#                                                              # öffentlichen Repos legen (Historie bleibt erhalten)
#
# Optionen:
#   --dry-run    Export nur in ein temporäres Verzeichnis, Dateiliste + Prüfung ausgeben, danach löschen.
#   --ref <rev>  Zu exportierender Stand (Standard: HEAD). Nur Committetes wird exportiert.
#   --force      Treffer der Prüfung auf interne Angaben nicht als Fehler werten (nur bewusst verwenden).
#   -m <text>    Commit-Nachricht für --into (Standard: „Lykos <version>“), z. B. für Fehlerbehebungen zwischen Releases.
#   --into <dir> Sauberer Klon des öffentlichen Repos: Inhalt wird durch den Export ersetzt und als
#                „Lykos <version>“ committet (Autor Zeasy Software <info@zeasy.software>). Nichts wird gepusht.
#
# Ausgelassen werden (siehe EXCLUDES): ops/ (Betrieb/Strategie), HANDOFF.md (Sitzungsnotizen), docs/handoff/.
# Danach wird der Export nach internen Mustern durchsucht (LAN-IPs, CT-Nummern, Konten, …); gitleaks läuft
# zusätzlich, falls installiert.
set -euo pipefail

EXCLUDES=(ops HANDOFF.md docs/handoff)

# Muster, die im öffentlichen Export nicht vorkommen dürfen (erweiterte Regex, Groß-/Kleinschreibung beachtet).
# Allgemeine Muster stehen hier; konkrete interne Namen (Hosts, Konten, Projekte) in ops/publish-forbidden.txt
# (eine Regex pro Zeile, # = Kommentar) – die Datei ist selbst intern und wird nicht exportiert.
FORBIDDEN=(
  '192\.168\.[0-9]{1,3}\.[0-9]{1,3}'
  '\bCT [0-9]{3}\b'
  'pct exec'
)
FORBIDDEN_FILE=ops/publish-forbidden.txt

DRY_RUN=0
FORCE=0
REF=HEAD
TARGET=''
INTO=''
MESSAGE=''
# Öffentliche Identität für Commits im öffentlichen Repo (keine persönlichen Adressen).
PUBLIC_NAME='Zeasy Software'
PUBLIC_EMAIL='info@zeasy.software'

usage() { sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    --ref) REF="${2:?--ref braucht einen Wert}"; shift ;;
    --into) INTO="${2:?--into braucht einen Klon}"; shift ;;
    -m) MESSAGE="${2:?-m braucht einen Text}"; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "Unbekannte Option: $1" >&2; usage >&2; exit 2 ;;
    *) [[ -z "$TARGET" ]] || { echo "Nur ein Zielverzeichnis erlaubt." >&2; exit 2; }; TARGET="$1" ;;
  esac
  shift
done

if [[ -n "$INTO" ]]; then
  [[ -z "$TARGET" && $DRY_RUN -eq 0 ]] || { echo "--into nicht zusammen mit Ziel oder --dry-run." >&2; exit 2; }
  git -C "$INTO" rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "$INTO ist kein Git-Klon." >&2; exit 1; }
  [[ -z "$(git -C "$INTO" status --porcelain)" ]] || { echo "$INTO hat uncommittete Änderungen." >&2; exit 1; }
  INTO="$(cd "$INTO" && pwd)"
elif [[ $DRY_RUN -eq 0 && -z "$TARGET" ]]; then
  echo "Zielverzeichnis fehlt (oder --dry-run verwenden)." >&2
  usage >&2
  exit 2
fi

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
COMMIT="$(git rev-parse --verify "$REF^{commit}")"
SHORT="$(git rev-parse --short "$COMMIT")"
VERSION="$(git show "$COMMIT:Cargo.toml" | sed -n 's/^version = "\(.*\)"$/\1/p' | head -n1)"

if [[ "$REF" == HEAD && -n "$(git status --porcelain)" ]]; then
  echo "Hinweis: uncommittete Änderungen werden NICHT exportiert." >&2
fi

PATHSPECS=(.)
for e in "${EXCLUDES[@]}"; do PATHSPECS+=(":(exclude)$e"); done

if [[ $DRY_RUN -eq 1 || -n "$INTO" ]]; then
  OUT="$(mktemp -d)"
  trap 'rm -rf "$OUT"' EXIT
else
  if [[ -e "$TARGET" && -n "$(ls -A "$TARGET" 2>/dev/null)" ]]; then
    echo "Ziel $TARGET existiert und ist nicht leer – bitte ein neues Verzeichnis angeben." >&2
    exit 1
  fi
  mkdir -p "$TARGET"
  OUT="$(cd "$TARGET" && pwd)"
fi

echo "==> Exportiere Lykos $VERSION ($SHORT) ohne: ${EXCLUDES[*]}"
git archive --format=tar "$COMMIT" -- "${PATHSPECS[@]}" | tar -x -C "$OUT"

FILES="$(cd "$OUT" && find . -type f | sed 's|^\./||' | sort)"
COUNT="$(printf '%s\n' "$FILES" | grep -c . || true)"
SIZE="$(du -sh "$OUT" | cut -f1)"

if [[ $DRY_RUN -eq 1 ]]; then
  echo "==> Dateien im Export ($COUNT, $SIZE):"
  # Übersicht: Dateien je Verzeichnis (zwei Ebenen)
  printf '%s\n' "$FILES" | awk -F/ '{ d = (NF > 2) ? $1"/"$2"/" : (NF > 1 ? $1"/" : $0); n[d]++ } END { for (d in n) printf "  %5d  %s\n", n[d], d }' | sort -k2
  echo "==> Ausgelassen:"
  for e in "${EXCLUDES[@]}"; do
    n="$(git ls-tree -r --name-only "$COMMIT" -- "$e" | grep -c . || true)"
    printf '  %5d  %s\n' "$n" "$e"
  done
fi

echo "==> Prüfe Export auf interne Angaben"
if [[ -f "$FORBIDDEN_FILE" ]]; then
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ -z "$line" || "$line" == \#* ]] && continue
    FORBIDDEN+=("$line")
  done < "$FORBIDDEN_FILE"
  echo "  ${#FORBIDDEN[@]} Muster (inkl. $FORBIDDEN_FILE)"
else
  echo "  ${#FORBIDDEN[@]} allgemeine Muster ($FORBIDDEN_FILE fehlt)"
fi
PATTERN="$(IFS='|'; echo "${FORBIDDEN[*]}")"
# Das Skript selbst enthält die allgemeinen Muster und wird nicht geprüft.
if HITS="$(cd "$OUT" && grep -rnIE --exclude=publish-public.sh "$PATTERN" . 2>/dev/null)"; then
  echo "$HITS" | sed 's|^\./|  |' | cut -c1-200 >&2
  if [[ $FORCE -eq 1 ]]; then
    echo "WARNUNG: interne Angaben gefunden (--force: wird ignoriert)." >&2
  else
    echo "FEHLER: interne Angaben im Export gefunden. Bereinigen oder mit --force bewusst übergehen." >&2
    [[ $DRY_RUN -eq 1 ]] || echo "Unvollständiger Export liegt in $OUT – bitte löschen." >&2
    exit 1
  fi
else
  echo "  keine Treffer"
fi

if command -v gitleaks >/dev/null 2>&1; then
  echo "==> gitleaks (Verzeichnis-Scan)"
  # Im Export-Verzeichnis laufen lassen: dann passen die relativen Pfade aus .gitleaksignore.
  if ! (cd "$OUT" && gitleaks dir --no-banner --redact .); then
    echo "WARNUNG: gitleaks meldet Funde – bitte prüfen (Testdaten sind oft Fehlalarme)." >&2
  fi
else
  echo "==> gitleaks nicht installiert – Secrets-Scan übersprungen"
fi

if [[ $DRY_RUN -eq 1 ]]; then
  echo "==> Trockenlauf beendet, nichts geschrieben."
  exit 0
fi

if [[ -n "$INTO" ]]; then
  echo "==> Übernehme Export als neuen Commit in $INTO"
  # Alles Versionierte ersetzen (.git bleibt): Gelöschte Dateien verschwinden so auch öffentlich.
  git -C "$INTO" rm -r -q --ignore-unmatch -- . >/dev/null
  (cd "$OUT" && tar -cf - .) | tar -xf - -C "$INTO"
  git -C "$INTO" add -A
  if git -C "$INTO" diff --cached --quiet; then
    echo "  keine Änderungen gegenüber dem öffentlichen Stand."
    exit 0
  fi
  git -C "$INTO" diff --cached --stat | tail -n 1
  git -C "$INTO" -c user.name="$PUBLIC_NAME" -c user.email="$PUBLIC_EMAIL" commit -q -m "${MESSAGE:-Lykos $VERSION}" \
    -m "Quellstand: $SHORT (nicht öffentliche Historie)."
  git -C "$INTO" log --oneline -1
  cat <<EOF

Fertig. Nichts wurde gepusht. Prüfen und veröffentlichen:
  git -C "$INTO" show --stat
  git -C "$INTO" push && git -C "$INTO" tag -a v$VERSION -m "Lykos $VERSION" && git -C "$INTO" push origin v$VERSION
  (Tag mit öffentlicher Identität: git -c user.name="$PUBLIC_NAME" -c user.email="$PUBLIC_EMAIL" -C … tag …)
EOF
  exit 0
fi

echo "==> Neue Historie in $OUT"
git -C "$OUT" init -q -b main
git -C "$OUT" add -A
git -C "$OUT" -c user.name="$PUBLIC_NAME" -c user.email="$PUBLIC_EMAIL" commit -q -m "Lykos $VERSION – erste öffentliche Version" \
  -m "Quellstand: $SHORT (nicht öffentliche Historie)."
git -C "$OUT" log --oneline -1

cat <<EOF

Fertig. Nichts wurde gepusht. Nächste Schritte (von Hand):
  1. Export prüfen:     cd "$OUT" && git show --stat | less
  2. Öffentliches Repo anlegen und Remote setzen: git -C "$OUT" remote add origin <url>
  3. Pushen:            git -C "$OUT" push -u origin main
  4. Tag setzen:        git -C "$OUT" tag v$VERSION && git -C "$OUT" push origin v$VERSION
EOF
