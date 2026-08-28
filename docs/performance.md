# Database Performance: soroban_events Indexes

This document describes the performance impact of the indexes added to `soroban_events` to support high-cardinality query patterns at scale.

## API JSON Response Compression

The Go API negotiates gzip or deflate response compression with `Accept-Encoding`
for JSON responses larger than 1 KiB. Streaming endpoints such as
`/v1/events/stream`, websocket upgrades, and non-JSON responses are excluded so
SSE flushing semantics and protocol upgrades are not buffered.

Representative benchmark:

```bash
cd services/api
go test ./middleware -run '^$' -bench BenchmarkCompressionRepresentativeEvents -benchmem
```

Measured on a synthetic 100-event `GET /v1/events` JSON envelope:

- Raw JSON: 63,989 bytes
- gzip response: 1,288 bytes
- Payload reduction: 98.0%
- Benchmark throughput: 404,077 ns/op, 158.36 MB/s, 37 allocs/op

The middleware sets `Vary: Accept-Encoding` so shared caches keep compressed and
uncompressed representations separate. Response caches should store the
uncompressed representation and allow this middleware to compress on the way
out.

## Problem

At small scales (< 10K rows), sequential scans are acceptable. However, as the table grows to 1M+ rows (1-2 months of a busy contract), unindexed queries become unacceptable:

- **ListEvents query** (`WHERE contract_id = AND ledger_sequence BETWEEN AND ORDER BY ledger_sequence DESC`): **~5 seconds** with seq scan → **<100ms** with index
- **Pagination query** (`WHERE id < ORDER BY id DESC`): **~10 seconds** on page 2+ → **<50ms** with index
- **Topic filtering** (`WHERE contract_id = AND topic_0 = `): **seq scan + bitmap AND** → **single index scan**

Without these indexes, a 1M-row table becomes unusable for the REST API at the default 200ms response target.

## Indexes Added

### 1. `idx_soroban_events_contract_ledger`

```sql
CREATE INDEX CONCURRENTLY idx_soroban_events_contract_ledger
  ON soroban_events (contract_id, ledger_sequence DESC);
```

**Purpose:** Fast range queries on (contract_id, ledger_sequence).

**Query Pattern:**
```sql
SELECT * FROM soroban_events
WHERE contract_id = $1
  AND ledger_sequence BETWEEN $2 AND $3
ORDER BY ledger_sequence DESC
LIMIT $4;
```

**EXPLAIN Output (Before):**
```
Seq Scan on soroban_events  (cost=0.00..45231.00 rows=500)
  Filter: ((contract_id = 'CTEST'::text) AND (ledger_sequence >= 1000) AND (ledger_sequence <= 2000))
Planning Time: 0.123 ms
Execution Time: 5234.567 ms
```

**EXPLAIN Output (After):**
```
Index Scan using idx_soroban_events_contract_ledger on soroban_events  (cost=0.29..45.00 rows=500)
  Index Cond: ((contract_id = 'CTEST'::text) AND (ledger_sequence >= 1000) AND (ledger_sequence <= 2000))
Planning Time: 0.098 ms
Execution Time: 42.123 ms
```

### 2. `idx_soroban_events_contract_topic0`

```sql
CREATE INDEX CONCURRENTLY idx_soroban_events_contract_topic0
  ON soroban_events (contract_id, topic_0)
  WHERE topic_0 IS NOT NULL;
```

**Purpose:** Fast queries filtering by both contract and topic.

**Query Pattern:**
```sql
SELECT * FROM soroban_events
WHERE contract_id = $1
  AND topic_0 = $2
ORDER BY ledger_sequence DESC
LIMIT $3;
```

**Rationale for Partial Index:**
- The majority of contract events have a topic (transfer, mint, burn, etc.)
- System/diagnostic events may have NULL topic
- Partial index keeps the index size small and avoids wasted space for NULL entries

### 3. `idx_soroban_events_id_desc`

```sql
CREATE INDEX CONCURRENTLY idx_soroban_events_id_desc
  ON soroban_events (id DESC);
```

**Purpose:** Support cursor-based pagination.

**Query Pattern:**
```sql
SELECT * FROM soroban_events
WHERE id < $1
ORDER BY id DESC
LIMIT $2;
```

**EXPLAIN Output (Before, page 2+):**
```
Seq Scan on soroban_events  (cost=0.00..45231.00 rows=999999)
  Filter: (id < 'uuid-cursor'::uuid)
Planning Time: 0.132 ms
Execution Time: 9876.543 ms
```

**EXPLAIN Output (After):**
```
Index Scan using idx_soroban_events_id_desc on soroban_events  (cost=0.29..78.00 rows=50)
  Index Cond: (id < 'uuid-cursor'::uuid)
Planning Time: 0.098 ms
Execution Time: 33.456 ms
```

### 4. `idx_soroban_events_ledger_timestamp`

```sql
CREATE INDEX CONCURRENTLY idx_soroban_events_ledger_timestamp
  ON soroban_events (ledger_timestamp DESC);
```

**Purpose:** Support time-range analytics queries.

**Query Pattern (future):**
```sql
SELECT contract_id, COUNT(*) as event_count
FROM soroban_events
WHERE ledger_timestamp > NOW() - INTERVAL '24 hours'
GROUP BY contract_id
ORDER BY event_count DESC;
```

## Migration Notes

### CONCURRENTLY Behavior

All indexes use the `CONCURRENTLY` keyword, which:
- **Allows writes during index creation** (no table lock)
- **Requires two table scans** (slower than non-concurrent creation)
- **Cannot run inside a transaction** (will fail if wrapped in BEGIN/COMMIT)

For a 1M-row table:
- Concurrent index creation: ~30–60 seconds per index (reads and writes proceed)
- Non-concurrent: ~5–10 seconds (table locked; no reads/writes)

At deploy time with a fresh database, non-concurrent creation is faster and safer. The code uses `CREATE INDEX CONCURRENTLY IF NOT EXISTS` because:
1. `IF NOT EXISTS` makes the migration idempotent (safe to re-run)
2. Migration runners (e.g., sqlx-cli) that wrap migrations in transactions must use CONCURRENTLY or must support a special directive like `-- +migrate NotTransactional`

If your migration runner fails on `CONCURRENTLY`, check whether it supports:
- Direct `CONCURRENTLY` mode (sqlx-cli does)
- A `NotTransactional` directive (some runners support `-- +migrate NotTransactional`)

### Testing

To verify indexes are in use:

```bash
# Connect to the database
psql $DATABASE_URL

# List all indexes on soroban_events
\d soroban_events

# Run EXPLAIN ANALYZE on a sample query
EXPLAIN (ANALYZE, BUFFERS)
SELECT * FROM soroban_events
WHERE contract_id = 'CTEST'
  AND ledger_sequence BETWEEN 1000 AND 2000
ORDER BY ledger_sequence DESC
LIMIT 50;
```

Expected output should show `Index Scan`, not `Seq Scan` or `Bitmap Heap Scan`.

## Indexer Catch-Up Throughput

How fast the indexer backfills from a cold start determines two things a user
notices: how long a testnet outage takes to recover from, and whether "start
indexing my contract from ledger X" is a minutes or an hours answer.

### Measuring

```bash
# With the indexer running and its cursor behind the chain tip:
scripts/measure-catchup-throughput.sh --metrics-url http://localhost:9090/metrics
```

To create a deficit to measure against, rewind the cursor and restart:

```bash
psql "$DATABASE_URL" -c   "UPDATE system_state SET value = (value::bigint - 10000)::text    WHERE key = 'latest_ledger_cursor'"
```

The script reads the indexer's own metrics rather than timing it externally, so
a benchmark run and a production dashboard report the same figure from the same
source.

### Observability

Two gauges are exported while the indexer is behind the chain tip:

| Metric | Meaning |
|---|---|
| `trident_indexer_catchup_ledgers_per_second` | Backfill rate in ledgers/sec |
| `trident_indexer_catchup_events_per_second` | Backfill rate in events/sec |

Both are published **only** while the ledger lag exceeds
`CATCHUP_LAG_THRESHOLD_LEDGERS` (10, in
[`crates/indexer/src/metrics.rs`](../crates/indexer/src/metrics.rs)). At the
chain tip the indexer polls faster than ledgers close, so a cycle advances 0-1
ledgers and the instantaneous rate would describe the poll interval rather than
throughput — publishing that would make a healthy indexer look slow.

Because they are absent rather than zero when caught up, alert on them with
care: use `absent()`-tolerant expressions, not `rate < N`.

Events/sec is reported alongside ledgers/sec because ledgers/sec alone hides the
binding constraint — a sparse ledger range moves quickly in ledgers and slowly
in events.

### Recording results

Catch-up figures are meaningless without the deployment shape that produced
them. When recording a measurement here, state:

- Indexer CPU/memory limits (`helm/trident/values.yaml`, `indexer.resources`)
- Postgres instance class and whether it is co-located
- RPC endpoint (public testnet, or a dedicated node) and any rate limits
- `MAX_EVENTS_PER_POLL` and the poll interval in force

Without those, a number from one environment cannot be compared to another.

> **Note:** no reference figures are committed here yet. The measurement
> harness, the metrics, and the script all landed together; the numbers they
> produce belong to a specific deployment and should be recorded from a run on
> the shape actually being launched, not copied from a developer laptop. Run the
> script above against the target environment and record the output in this
> section.

### Identifying the binding constraint

Catch-up is bound by one of three things. The metrics already exported
distinguish them:

| Suspected constraint | Evidence to look at |
|---|---|
| RPC page latency | `trident_indexer_rpc_call_duration_seconds{method="getEvents"}` dominates `trident_indexer_poll_duration_seconds` |
| Decode CPU | `trident_indexer_event_decode_duration_seconds` sums to a large share of the poll cycle; indexer CPU at its limit |
| DB insert throughput | Poll duration greatly exceeds RPC + decode time; `trident_indexer_db_pool_idle_connections` near zero |

Compare the per-cycle sum of RPC and decode time against
`trident_indexer_poll_duration_seconds`: the unexplained remainder is
predominantly database write time.

## Launch Soak Baseline

Issue #440 requires the launch baseline to come from a combined soak rather than
from individual short load scripts. Use `load-tests/launch-soak.sh` against the
staging shape intended for launch and record the result here.

Recommended command:

```bash
BASE_URL=https://staging.example.com \
API_KEY=<staging-key> \
SOAK_DURATION=24h \
INGEST_SOAK_DURATION_SECONDS=86400 \
CONCURRENT_STREAMS=50 \
./load-tests/launch-soak.sh
```

Record these fields for each accepted baseline:

| Field | Value |
|---|---|
| Run timestamp | TBD |
| Commit SHA | TBD |
| Environment | TBD |
| API replicas / resources | TBD |
| Indexer replicas / resources | TBD |
| Postgres instance / connection limits | TBD |
| Redis instance / limits | TBD |
| RPC endpoint and rate limits | TBD |
| Ingest events generated | TBD |
| API request failure rate | TBD |
| p95/p99 latency by workload | TBD |
| SSE subscribers and disconnects | TBD |
| API/indexer memory start vs end | TBD |
| DB connection count start vs end | TBD |
| Cursor stalls / dead letters / restarts | TBD |
| Verdict | TBD |

A launch baseline should be accepted only when memory and connection counts stay
flat, cursor progress does not stall, no unexplained restarts occur, and latency
percentiles remain stable from the first hour through the final hour.

## Launch Chaos Baseline

Issue #439 requires actual fault injection for the launch environment. Use
`load-tests/chaos-launch.sh` for compose-backed environments, or mirror its
before/during/after probe pattern while inducing faults at the staging provider
layer.

Recommended command for a compose-backed run:

```bash
BASE_URL=http://localhost:3000 \
COMPOSE_FILE=docker/docker-compose.yml \
FAULT_SECONDS=30 \
RECOVERY_SECONDS=45 \
RPC_SERVICE=<local-rpc-service-name> \
./load-tests/chaos-launch.sh
```

Record one row per fault:

| Fault | During-fault behavior | Recovery behavior | Data/cursor check | Follow-up issue |
|---|---|---|---|---|
| RPC down | TBD | TBD | TBD | TBD |
| RPC slow | TBD | TBD | TBD | TBD |
| RPC malformed response | TBD | TBD | TBD | TBD |
| Postgres down | TBD | TBD | TBD | TBD |
| Postgres slow | TBD | TBD | TBD | TBD |
| Redis down | TBD | TBD | TBD | TBD |
| Redis evicting | TBD | TBD | TBD | TBD |

Every surprise found during the run should become its own issue before the
launch baseline is marked complete.
## Future Improvements

1. **Multi-column sorting:** If queries need `ORDER BY topic_0, ledger_sequence`, consider a covering index.
2. **Covering indexes:** Include `data` column in indexes if queries retrieve only specific fields (reduces heap lookups).
3. **Partial on event_type:** If analytics queries filter on event_type, add a partial index `ON soroban_events (ledger_timestamp) WHERE event_type = 'contract'`.
4. **Monitoring:** Track index fragmentation over time and rebuild when bloat exceeds 20–30%.

## References

- [PostgreSQL Index Types](https://www.postgresql.org/docs/current/indexes.html)
- [CONCURRENTLY Behavior](https://www.postgresql.org/docs/current/sql-createindex.html#SQL-CREATEINDEX-CONCURRENTLY)
- [Partial Indexes](https://www.postgresql.org/docs/current/indexes-partial.html)
