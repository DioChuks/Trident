#!/usr/bin/env bash
# Launch chaos verification harness for issue #439.
#
# Exercises the major degradation assumptions by inducing RPC, Postgres, and
# Redis faults against a running compose-backed environment, then checking that
# the API reports a degraded state during the fault and recovers afterwards.
#
# This script intentionally records observations instead of hiding them behind a
# single pass/fail assertion. Surprises should be promoted into follow-up issues.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_ID="$(date -u +%Y%m%d-%H%M%S)"
OUT_DIR="${SCRIPT_DIR}/chaos-results/${RUN_ID}"
mkdir -p "$OUT_DIR"

BASE_URL="${BASE_URL:-http://localhost:3000}"
COMPOSE_FILE="${COMPOSE_FILE:-docker/docker-compose.yml}"
COMPOSE_PROJECT="${COMPOSE_PROJECT:-}"
POSTGRES_SERVICE="${POSTGRES_SERVICE:-postgres}"
REDIS_SERVICE="${REDIS_SERVICE:-redis}"
RPC_SERVICE="${RPC_SERVICE:-}"
FAULT_SECONDS="${FAULT_SECONDS:-30}"
RECOVERY_SECONDS="${RECOVERY_SECONDS:-45}"

compose() {
  if [ -n "$COMPOSE_PROJECT" ]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" "$@"
  else
    docker compose -f "$COMPOSE_FILE" "$@"
  fi
}

probe() {
  local label="$1"
  local path="${2:-/v1/ready}"
  local body="${OUT_DIR}/${label}.body"
  local status
  status="$(curl -sS -o "$body" -w '%{http_code}' "${BASE_URL}${path}" || true)"
  printf '%s,%s,%s,%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$label" "$path" "$status" \
    | tee -a "${OUT_DIR}/probes.csv"
}

record_ps() {
  local scenario="$1"
  compose ps >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
}

run_stop_scenario() {
  local scenario="$1"
  local service="$2"
  echo "=== ${scenario} ===" | tee -a "${OUT_DIR}/summary.txt"
  probe "${scenario}-before"
  echo "[$scenario] stopping ${service}" | tee -a "${OUT_DIR}/${scenario}.log"
  compose stop "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep 5
  probe "${scenario}-during"
  sleep "$FAULT_SECONDS"
  echo "[$scenario] starting ${service}" | tee -a "${OUT_DIR}/${scenario}.log"
  compose start "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep "$RECOVERY_SECONDS"
  probe "${scenario}-after"
  record_ps "$scenario"
}

run_pause_scenario() {
  local scenario="$1"
  local service="$2"
  echo "=== ${scenario} ===" | tee -a "${OUT_DIR}/summary.txt"
  probe "${scenario}-before"
  echo "[$scenario] pausing ${service}" | tee -a "${OUT_DIR}/${scenario}.log"
  compose pause "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep 5
  probe "${scenario}-during"
  sleep "$FAULT_SECONDS"
  echo "[$scenario] unpausing ${service}" | tee -a "${OUT_DIR}/${scenario}.log"
  compose unpause "$service" >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  sleep "$RECOVERY_SECONDS"
  probe "${scenario}-after"
  record_ps "$scenario"
}

run_redis_evicting() {
  local scenario="redis-evicting"
  echo "=== ${scenario} ===" | tee -a "${OUT_DIR}/summary.txt"
  probe "${scenario}-before"
  echo "[$scenario] flushing Redis DB" | tee -a "${OUT_DIR}/${scenario}.log"
  compose exec -T "$REDIS_SERVICE" redis-cli FLUSHDB >> "${OUT_DIR}/${scenario}.log" 2>&1 || true
  probe "${scenario}-during"
  sleep "$RECOVERY_SECONDS"
  probe "${scenario}-after"
  record_ps "$scenario"
}

echo "timestamp,label,path,status" > "${OUT_DIR}/probes.csv"
echo "Trident launch chaos run ${RUN_ID}" > "${OUT_DIR}/summary.txt"
echo "BASE_URL=${BASE_URL}" >> "${OUT_DIR}/summary.txt"
echo "COMPOSE_FILE=${COMPOSE_FILE}" >> "${OUT_DIR}/summary.txt"

run_stop_scenario postgres-down "$POSTGRES_SERVICE"
run_pause_scenario postgres-slow "$POSTGRES_SERVICE"
run_stop_scenario redis-down "$REDIS_SERVICE"
run_redis_evicting

if [ -n "$RPC_SERVICE" ]; then
  run_stop_scenario rpc-down "$RPC_SERVICE"
  run_pause_scenario rpc-slow "$RPC_SERVICE"
else
  cat <<'EOF' | tee -a "${OUT_DIR}/summary.txt"
RPC_SERVICE is not set, so rpc-down and rpc-slow were not induced automatically.
Set RPC_SERVICE to the compose service name for a local RPC container, or run the
same before/during/after probes while applying the fault at the network/provider layer.
EOF
fi

cat <<EOF | tee -a "${OUT_DIR}/summary.txt"

Review checklist:
- Confirm /v1/ready degraded during each dependency outage instead of hanging.
- Confirm /v1/ready recovered after each dependency returned.
- Check API and indexer logs for cursor corruption, data loss, or unbounded retries.
- Promote every unexpected behavior into its own follow-up issue.
EOF

echo "Chaos run complete. Results: ${OUT_DIR}"