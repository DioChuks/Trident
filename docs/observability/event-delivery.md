# Event delivery: outbox, at-least-once semantics and alerting

The indexer does not publish events to Redis inline with the poll loop. It
commits each event to Postgres **together with an `event_outbox` row in the same
transaction**, and a relay task publishes unpublished rows to the
`trident:events` stream (issue #200).

## Why

Writing to Postgres and then publishing to Redis as two independent steps loses
events. If the process dies — or Redis errors — after the commit but before the
`XADD`, the event exists in Postgres but never reaches live subscribers, and
there is no replay path. The gap is only visible to a client that also polls
REST, which is exactly the kind of silent hole that erodes trust in a real-time
product.

With the outbox, a committed event always carries a delivery record, so the
relay picks it up on the next pass after a restart.

## Delivery semantics: at-least-once

The relay publishes a row and then marks it published. A crash between those two
steps re-delivers the event on the next pass. **Exactly-once is not the target.**

**Consumers must dedupe by event id.** Every stream entry carries an `event_id`
field: the deterministic UUIDv5 derived from
`(contract_id, ledger_sequence, event_index)`. The same logical event always
produces the same `event_id`, so a consumer that tracks recently seen ids can
discard a repeat safely. The same id is the primary key of the `soroban_events`
row, so REST and stream data can be correlated directly.

Ordering within the stream follows the outbox `seq`. A batch stops at the first
publish failure and only the rows published before it are marked, so the next
pass resumes at the failed row rather than skipping past it.

## Metrics

| Metric | Type | Meaning |
|---|---|---|
| `trident_indexer_outbox_backlog` | gauge | Committed events not yet published |
| `trident_indexer_outbox_published_total` | counter | Events delivered to the stream by the relay |
| `trident_indexer_outbox_publish_failures_total` | counter | Failed publish attempts |
| `trident_indexer_rpc_timeouts_total` | counter | RPC calls aborted by the connect or request timeout |
| `trident_indexer_rpc_active_endpoint` | gauge | Index of the RPC endpoint in use, `0` = primary |
| `trident_indexer_rpc_failovers_total` | counter | Switches to a different RPC endpoint |

## Alerting

A healthy relay keeps `trident_indexer_outbox_backlog` near zero. A backlog that
grows without recovering means live subscribers are missing data, even though
Postgres is up to date.

```yaml
- alert: TridentOutboxBacklogGrowing
  expr: trident_indexer_outbox_backlog > 10000
  for: 5m
  annotations:
    summary: "Outbox backlog above threshold — live subscribers are falling behind"
```

`OUTBOX_BACKLOG_ALERT_THRESHOLD` (default `10000`) controls the matching
warning the relay logs, so the log line and the alert fire on the same
condition. Tune both together.

A sustained non-zero `trident_indexer_rpc_active_endpoint` is worth alerting on
as well: the indexer is running on a fallback provider and the primary has not
recovered.
