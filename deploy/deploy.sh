#!/usr/bin/env bash
# Deployt den aktuellen Git-Stand (HEAD) auf einen Docker-Host und startet den Stack neu.
# Nutzung: deploy/deploy.sh [ssh-ziel]   (z. B. root@chat.example.org)
#   Ohne Argument gilt $LYKOS_DEPLOY_TARGET bzw. LYKOS_DEPLOY_TARGET aus ops/deploy.env (falls vorhanden).
#
# Code liegt unter /opt/lykos und wird bei jedem Deploy komplett ersetzt.
# Secrets liegen getrennt in /etc/lykos/lykos.env und werden beim ersten Lauf erzeugt.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

if [[ -z "${1:-}" && -z "${LYKOS_DEPLOY_TARGET:-}" && -f ops/deploy.env ]]; then
  # shellcheck disable=SC1091
  source ops/deploy.env
fi
TARGET="${1:-${LYKOS_DEPLOY_TARGET:-}}"
if [[ -z "$TARGET" ]]; then
  echo "Nutzung: deploy/deploy.sh <ssh-ziel>   (oder LYKOS_DEPLOY_TARGET setzen)" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "Warnung: uncommittete Änderungen werden NICHT mit deployt." >&2
fi

echo "==> Übertrage $(git rev-parse --short HEAD) nach $TARGET:/opt/lykos"
ssh "$TARGET" "rm -rf /opt/lykos && mkdir -p /opt/lykos"
git archive --format=tar HEAD | ssh "$TARGET" "tar -x -C /opt/lykos"

echo "==> Secrets prüfen und Stack starten"
ssh "$TARGET" bash -s <<'REMOTE'
set -euo pipefail
ENV_FILE=/etc/lykos/lykos.env
if [[ ! -f $ENV_FILE ]]; then
  install -d -m 700 /etc/lykos
  sed -e "s|^POSTGRES_PASSWORD=.*|POSTGRES_PASSWORD=$(openssl rand -hex 24)|" \
      -e "s|^LIVEKIT_API_KEY=.*|LIVEKIT_API_KEY=lk$(openssl rand -hex 6)|" \
      -e "s|^LIVEKIT_API_SECRET=.*|LIVEKIT_API_SECRET=$(openssl rand -hex 32)|" \
      /opt/lykos/deploy/.env.example > $ENV_FILE
  chmod 600 $ENV_FILE
  echo "Neue $ENV_FILE mit generierten Secrets angelegt."
fi
# Ordner für Impressum/Datenschutzerklärung (im Container read-only unter /app/legal, siehe .env.example).
install -d -m 755 /etc/lykos/legal
# Neue Variablen aus der Vorlage ergänzen; bestehende Werte bleiben unangetastet.
while IFS= read -r line; do
  [[ $line =~ ^([A-Z_]+)= ]] || continue
  grep -q "^${BASH_REMATCH[1]}=" $ENV_FILE || { echo "$line" >> $ENV_FILE; echo "$ENV_FILE: ${BASH_REMATCH[1]} ergänzt."; }
done < /opt/lykos/deploy/.env.example
cd /opt/lykos/deploy
# LiveKit liest keine Umgebungsvariablen in der YAML: Webhook-Key einsetzen (der Key ist nicht geheim).
LIVEKIT_API_KEY=$(grep -E '^LIVEKIT_API_KEY=' $ENV_FILE | cut -d= -f2-)
sed -i "s|__LIVEKIT_API_KEY__|${LIVEKIT_API_KEY}|" livekit.yaml
export LIVEKIT_CONFIG_HASH=$(sha256sum livekit.yaml | cut -c1-16)
docker compose --env-file $ENV_FILE up -d --build --remove-orphans
docker compose --env-file $ENV_FILE ps
REMOTE
