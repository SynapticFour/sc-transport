#!/usr/bin/env sh
set -e

COMPOSE_FILE="docker-compose.prod.yml"
OFFLINE=0

for arg in "$@"; do
  case "$arg" in
    --offline) OFFLINE=1 ;;
  esac
done

[ -f "$COMPOSE_FILE" ] || { echo "ERROR: ${COMPOSE_FILE} not found."; exit 1; }
[ -f sct.toml ] || { echo "ERROR: sct.toml fehlt. cp sct.toml.example sct.toml"; exit 1; }

if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

if [ -z "${SCT_VERSION:-}" ]; then
  echo "ERROR: SCT_VERSION ist nicht gesetzt. Bitte .env konfigurieren."
  echo "       Beispiel: cp .env.example .env  und  SCT_VERSION=v0.1.0 setzen"
  exit 1
fi

export SCT_VERSION
command -v docker >/dev/null || { echo "ERROR: Docker nicht gefunden."; exit 1; }

echo "[sc-transport] Install (Version ${SCT_VERSION})..."

if [ "$OFFLINE" = "1" ]; then
  docker compose -f "$COMPOSE_FILE" pull --ignore-buildable 2>/dev/null || true
  docker compose -f "$COMPOSE_FILE" up -d --pull never
else
  docker compose -f "$COMPOSE_FILE" pull
  docker compose -f "$COMPOSE_FILE" up -d
fi

API="${SCT_API_PORT:-7273}"
i=0
while [ "$i" -lt 24 ]; do
  if curl -sf "http://localhost:${API}/v1/health" >/dev/null 2>&1; then
    echo "[sc-transport] Daemon bereit (API port ${API})"
    exit 0
  fi
  i=$((i + 1))
  sleep 5
done

echo "ERROR: sc-transport API nicht erreichbar."
docker compose -f "$COMPOSE_FILE" logs sct-daemon 2>&1 | tail -40 || true
exit 1
