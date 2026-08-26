#!/usr/bin/env bash
# Verify that every metric name referenced in Prometheus alert rules actually
# exists in the live /metrics endpoints (issue #393).
#
# Usage:
#   ./scripts/verify-alert-metrics.sh <api-metrics-url> <indexer-metrics-url>
#
# Example (local docker compose):
#   ./scripts/verify-alert-metrics.sh \
#     http://localhost:3000/metrics \
#     http://localhost:9090/metrics
#
# Example (staging):
#   ./scripts/verify-alert-metrics.sh \
#     https://api-staging.trident.example/metrics \
#     https://indexer-staging.trident.example/metrics
#
# Exit codes:
#   0 - all referenced metrics exist
#   1 - one or more metrics are missing
#   2 - usage error or connectivity failure

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <api-metrics-url> <indexer-metrics-url>" >&2
  echo "" >&2
  echo "Example:" >&2
  echo "  $0 http://localhost:3000/metrics http://localhost:9090/metrics" >&2
  exit 2
fi

API_METRICS_URL="$1"
INDEXER_METRICS_URL="$2"

echo "=== Fetching live metrics ==="
API_METRICS=$(curl --silent --fail --max-time 10 "$API_METRICS_URL" || {
  echo "ERROR: failed to fetch $API_METRICS_URL" >&2
  exit 2
})
INDEXER_METRICS=$(curl --silent --fail --max-time 10 "$INDEXER_METRICS_URL" || {
  echo "ERROR: failed to fetch $INDEXER_METRICS_URL" >&2
  exit 2
})

echo "✓ Fetched API metrics ($(echo "$API_METRICS" | wc -l) lines)"
echo "✓ Fetched indexer metrics ($(echo "$INDEXER_METRICS" | wc -l) lines)"

# Extract metric names (lines that don't start with # and contain a metric name)
# Format: metric_name{labels} value timestamp OR metric_name value timestamp
# Both payloads must go through the same extraction. Without the braces the
# first `echo` is its own command and the API metrics reach EMITTED_METRICS
# raw — values and `# HELP`/`# TYPE` lines included — so nothing matches by
# name and the comparison below silently misbehaves.
EMITTED_METRICS=$( { echo "$API_METRICS"; echo "$INDEXER_METRICS"; } \
  | grep -v '^#' \
  | grep -v '^$' \
  | sed -E 's/^([a-zA-Z_:][a-zA-Z0-9_:]*).*/\1/' \
  | sort -u)

echo ""
echo "=== Extracting metric names from alert rules ==="

# Extract all metric names from alert YAML files. This regex captures:
# - Metric names in PromQL expressions (alphanumeric + underscores + colons)
# - Ignores Prometheus functions (e.g., rate, sum, avg_over_time)
# - Captures metric names before { or [ or ( or space
extract_metrics_from_alerts() {
  local alert_file="$1"
  # Extract PromQL expressions from expr: lines and recording rule expr: lines
  # Then extract all potential metric names (word characters that could be metrics)
  # Filter out Prometheus functions and operators
  grep -E '^\s+expr:' "$alert_file" \
    | sed 's/expr://g' \
    | grep -oE '\b[a-zA-Z_:][a-zA-Z0-9_:]*\b' \
    | grep -v -E '^(rate|irate|increase|sum|avg|min|max|count|stddev|stdvar|topk|bottomk|quantile|histogram_quantile|time|bool|by|and|or|unless|on|ignoring|group_left|group_right|offset|without|avg_over_time|min_over_time|max_over_time|sum_over_time|count_over_time|quantile_over_time|stddev_over_time|stdvar_over_time|last_over_time|present_over_time|absent|absent_over_time|changes|deriv|predict_linear|delta|idelta|resets|floor|ceil|round|exp|ln|log2|log10|sqrt|abs|le|humanizeDuration|humanizePercentage|for|labels|annotations|alert|record|if|else|end)$' \
    | sort -u
}

ALERT_FILES=(
  "monitoring/alerts.yml"
  "observability/burn-rate-alerts.yml"
  "observability/rpc-alerts.yml"
)

REFERENCED_METRICS=()
for alert_file in "${ALERT_FILES[@]}"; do
  if [ ! -f "$alert_file" ]; then
    echo "WARNING: $alert_file not found, skipping" >&2
    continue
  fi
  echo "Extracting from $alert_file..."
  while IFS= read -r metric; do
    REFERENCED_METRICS+=("$metric")
  done < <(extract_metrics_from_alerts "$alert_file")
done

# Deduplicate
REFERENCED_METRICS=($(printf '%s\n' "${REFERENCED_METRICS[@]}" | sort -u))

echo "✓ Found ${#REFERENCED_METRICS[@]} unique metric names referenced in alerts"
echo ""

echo "=== Verifying metric existence ==="
MISSING_METRICS=()

for metric in "${REFERENCED_METRICS[@]}"; do
  # Check if this metric name exists in the emitted metrics
  # Use grep -F for fixed string match (no regex interpretation)
  # -x anchors to the whole line. Without it a referenced name matches any
  # emitted name that merely contains it, so a metric nothing exports still
  # passes as long as some longer series shares the prefix — which would let
  # exactly the drift this script exists to catch through.
  if printf '%s\n' "$EMITTED_METRICS" | grep -qxF "$metric"; then
    echo "✓ $metric"
  else
    echo "✗ $metric (NOT FOUND in live /metrics)"
    MISSING_METRICS+=("$metric")
  fi
done

echo ""
if [ ${#MISSING_METRICS[@]} -eq 0 ]; then
  echo "SUCCESS: All ${#REFERENCED_METRICS[@]} referenced metrics exist in live /metrics endpoints"
  exit 0
else
  echo "FAILURE: ${#MISSING_METRICS[@]} metric(s) referenced by alerts do not exist:" >&2
  for metric in "${MISSING_METRICS[@]}"; do
    echo "  - $metric" >&2
  done
  echo "" >&2
  echo "These metrics are queried by alert rules but are never emitted." >&2
  echo "The alerts referencing them will never fire (PromQL evaluates to empty)." >&2
  exit 1
fi
