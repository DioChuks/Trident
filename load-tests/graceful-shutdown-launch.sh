#!/usr/bin/env bash
# Rolling shutdown verification harness for issue #442.
#
# Runs API and SSE load while terminating the API and indexer services with
# SIGTERM. The resulting logs make it clear whether readiness recovers, requests
# fail, SSE clients reconnect cleanly, and the indexer exits without an
# ambiguous cursor.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_ID="$(date -u +%Y%m%d-%H%M%S)"
OUT_DIR="${SCRIPT_DIR}/shutdown-results/${RUN_ID}"
mkdir -p "$OUT_DIR"

BASE_URL="${BASE_URL:-http://localhost:3000}"
COMPOSE_FILE="${COMPOSE_FILE:-docker/docker-compose.yml}"
COMPOSE_PROJECT="${COMPOSE_PROJECT:-}"
API_SERVICE="${API_SERVICE:-api}"
INDEXER_SERVICE="${INDEXER_SERVICE:-indexer}"
DRAIN_SECONDS="${DRAIN_SECONDS:-30}"
RECOVERY_SECONDS="${RECOVERY_SECONDS:-45}"
API_LOAD_DURATION="${API_LOAD_DURATION:-2m}"
CONCURRENT_STREAMS="${CONCURRENT_STREAMS:-20}"
HOLD_SECONDS="${HOLD_SECONDS:-90}"
API_KEY="${API_KEY:-}"

export BASE_URL API_KEY

compose() {
  if [ -n "$COMPOSE_PROJECT" ]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" "$@"
  else
    docker compose -f "$COMPOSE_FILE" "$@"
  fi
}

probe_ready() {
  local label="$1"
  local status
  status="$(curl -sS -o "${OUT_DIR}/${label}.body" -w '%{http_code}' "${BASE_URL}/v1/ready" || true)"
  printf '%s,%s,%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$label" "$status" \
    | tee -a "${OUT_DIR}/ready.csv"
}

start_load() {
  env LIST_VUS=10 GET_VUS=5 DURATION="$API_LOAD_DURATION" \
    k6 run "${SCRIPT_DIR}/events-load.js" > "${OUT_DIR}/events-load.log" 2>&1 &
  echo $! > "${OUT_DIR}/events-load.pid"

  env CONCURRENT_STREAMS="$CONCURRENT_STREAMS" HOLD_SECONDS="$HOLD_SECONDS" \
    k6 run "${SCRIPT_DIR}/stream-load.js" > "${OUT_DIR}/stream-load.log" 2>&1 &
  echo $! > "${OUT_DIR}/stream-load.pid"
}

terminate_service() {
  local scenario="$1"
  local service="$2"
  echo "=== ${scenario}: SIGTERM ${service} ===" | tee -a "${OUT_DIR}/summary.txt"
  probe_ready "${scenario}-before"
  compose kill -s SIGTERM "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep "$DRAIN_SECONDS"
  probe_ready "${scenario}-during-drain"
  compose up -d "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep "$RECOVERY_SECONDS"
  probe_ready "${scenario}-after-recovery"
  compose logs --no-color --tail=200 "$service" > "${OUT_DIR}/${scenario}.service.log" 2>&1 || true
}

echo "timestamp,label,status" > "${OUT_DIR}/ready.csv"
echo "Trident rolling shutdown run ${RUN_ID}" > "${OUT_DIR}/summary.txt"
echo "BASE_URL=${BASE_URL}" >> "${OUT_DIR}/summary.txt"
echo "COMPOSE_FILE=${COMPOSE_FILE}" >> "${OUT_DIR}/summary.txt"

start_load
sleep 5
terminate_service api-shutdown "$API_SERVICE"
terminate_service indexer-shutdown "$INDEXER_SERVICE"

status=0
for pid_file in "${OUT_DIR}"/*.pid; do
  [ -e "$pid_file" ] || continue
  name="$(basename "$pid_file" .pid)"
  pid="$(cat "$pid_file")"
  if wait "$pid"; then
    echo "${name}: completed" | tee -a "${OUT_DIR}/summary.txt"
  else
    echo "${name}: failed during shutdown run" | tee -a "${OUT_DIR}/summary.txt"
    status=1
  fi
done

cat <<EOF | tee -a "${OUT_DIR}/summary.txt"

Review checklist:
- Confirm in-flight API requests drained or returned intentional 503s during SIGTERM.
- Confirm SSE clients did not hang silently and can reconnect with Last-Event-ID.
- Confirm indexer logs show cursor commit or an explicit safe retry point before exit.
- Confirm Kubernetes terminationGracePeriodSeconds/preStop settings exceed measured drain time.
EOF

echo "Shutdown verification complete. Results: ${OUT_DIR}"
exit "$status"