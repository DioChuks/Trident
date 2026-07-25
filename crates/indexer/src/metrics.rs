//! Prometheus metrics for the indexer, served from a `GET /metrics` HTTP
//! endpoint (default port 9090, configurable via `METRICS_PORT`).
//!
//! [`install`] sets up the global recorder and starts the HTTP listener; the
//! `record_*`/`set_*` helpers below are called from the streamer at the
//! relevant points in `poll_once`.

use std::net::SocketAddr;

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use trident_common::TridentError;

pub const LEDGER_LAG: &str = "trident_indexer_ledger_lag";
pub const EVENTS_TOTAL: &str = "trident_indexer_events_total";
pub const EVENTS_SKIPPED_TOTAL: &str = "trident_indexer_events_skipped_total";
pub const PARSE_ERRORS_TOTAL: &str = "trident_indexer_parse_errors_total";
pub const POLL_DURATION_SECONDS: &str = "trident_indexer_poll_duration_seconds";
pub const POLL_ERRORS_TOTAL: &str = "trident_indexer_poll_errors_total";
pub const RPC_RETRIES_TOTAL: &str = "trident_indexer_rpc_retries_total";
pub const EFFECTIVE_POLL_INTERVAL_MS: &str = "trident_indexer_effective_poll_interval_ms";
pub const RPC_TIMEOUTS_TOTAL: &str = "trident_indexer_rpc_timeouts_total";
pub const RPC_ACTIVE_ENDPOINT: &str = "trident_indexer_rpc_active_endpoint";
pub const RPC_FAILOVERS_TOTAL: &str = "trident_indexer_rpc_failovers_total";
pub const OUTBOX_BACKLOG: &str = "trident_indexer_outbox_backlog";
pub const OUTBOX_PUBLISHED_TOTAL: &str = "trident_indexer_outbox_published_total";
pub const OUTBOX_PUBLISH_FAILURES_TOTAL: &str = "trident_indexer_outbox_publish_failures_total";
/// Unix timestamp (seconds) of the most recent completed poll cycle. Use
/// `time() - trident_indexer_last_poll_timestamp_seconds > N` as a
/// dead-man's-switch alert for a stalled indexer (#218).
pub const HEARTBEAT_TIMESTAMP: &str = "trident_indexer_last_poll_timestamp_seconds";
/// Bounded per-contract event counter. Labels: `contract` (allowlisted contract ID or `"other"`).
/// Cardinality: |allowlist| + 1. In index-all mode (no allowlist) all events land in `"other"`.
pub const EVENTS_BY_CONTRACT_TOTAL: &str = "trident_indexer_events_by_contract_total";
pub const EVENT_DECODE_DURATION_SECONDS: &str = "trident_indexer_event_decode_duration_seconds";

/// Install the global Prometheus recorder and start serving `/metrics` on
/// `port`. Must be called once, before the streamer starts recording.
pub fn install(port: u16) -> Result<(), TridentError> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();

    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
        .map_err(|e| {
            TridentError::config(anyhow::Error::new(e).context("failed to start metrics exporter"))
        })?;

    describe_gauge!(
        LEDGER_LAG,
        "Difference between chain tip and indexer cursor (ledgers)"
    );
    describe_counter!(EVENTS_TOTAL, "Total events processed since startup");
    describe_counter!(
        EVENTS_SKIPPED_TOTAL,
        "Events skipped (diagnostic, failed call, or contract filter)"
    );
    describe_counter!(PARSE_ERRORS_TOTAL, "Total events that failed XDR decoding");
    describe_histogram!(
        POLL_DURATION_SECONDS,
        "Time per poll_once cycle, in seconds"
    );
    describe_counter!(POLL_ERRORS_TOTAL, "Poll cycles that returned an error");
    describe_counter!(
        RPC_RETRIES_TOTAL,
        "Total RPC retries triggered by transient failures"
    );
    describe_gauge!(
        EFFECTIVE_POLL_INTERVAL_MS,
        "Current adaptive poll interval in milliseconds (issue #198)"
    );
    describe_counter!(
        RPC_TIMEOUTS_TOTAL,
        "RPC calls aborted by the connect or request timeout (issue #214)"
    );
    describe_gauge!(
        RPC_ACTIVE_ENDPOINT,
        "Index of the RPC endpoint currently in use, 0 = primary (issue #213)"
    );
    describe_counter!(
        RPC_FAILOVERS_TOTAL,
        "Times the indexer failed over to another RPC endpoint (issue #213)"
    );
    describe_gauge!(
        OUTBOX_BACKLOG,
        "Committed events not yet published to the Redis stream (issue #200)"
    );
    describe_counter!(
        OUTBOX_PUBLISHED_TOTAL,
        "Events published to the Redis stream by the outbox relay (issue #200)"
    );
    describe_counter!(
        OUTBOX_PUBLISH_FAILURES_TOTAL,
        "Outbox publish attempts that failed (issue #200)"
    );
    describe_gauge!(
        HEARTBEAT_TIMESTAMP,
        "Unix timestamp (seconds) of the most recent completed poll cycle (#218)"
    );
    describe_counter!(
        EVENTS_BY_CONTRACT_TOTAL,
        "Events processed per contract (bounded: allowlisted contract IDs + 'other' bucket)"
    );
    describe_histogram!(
        EVENT_DECODE_DURATION_SECONDS,
        "Time to XDR-decode a single event, in seconds (per-event parse latency)"
    );

    // Counters only render in the scrape output once touched at least once;
    // seed them at zero so /metrics is complete from the very first scrape.
    counter!(EVENTS_TOTAL).increment(0);
    counter!(EVENTS_SKIPPED_TOTAL).increment(0);
    counter!(PARSE_ERRORS_TOTAL).increment(0);
    counter!(POLL_ERRORS_TOTAL).increment(0);
    counter!(RPC_RETRIES_TOTAL).increment(0);
    counter!(RPC_TIMEOUTS_TOTAL).increment(0);
    counter!(RPC_FAILOVERS_TOTAL).increment(0);
    counter!(OUTBOX_PUBLISHED_TOTAL).increment(0);
    counter!(OUTBOX_PUBLISH_FAILURES_TOTAL).increment(0);
    gauge!(RPC_ACTIVE_ENDPOINT).set(0.0);
    gauge!(OUTBOX_BACKLOG).set(0.0);
    gauge!(LEDGER_LAG).set(0.0);
    gauge!(EFFECTIVE_POLL_INTERVAL_MS).set(0.0);
    gauge!(HEARTBEAT_TIMESTAMP).set(0.0);

    tracing::info!(port, "Metrics endpoint listening");
    Ok(())
}

pub fn set_ledger_lag(lag: i64) {
    gauge!(LEDGER_LAG).set(lag as f64);
}

pub fn set_effective_poll_interval(ms: u64) {
    gauge!(EFFECTIVE_POLL_INTERVAL_MS).set(ms as f64);
}

/// Stamp the heartbeat to the current Unix time. Called at the end of every
/// poll cycle (success or failure) so a dead-man's switch alert can detect a
/// stalled-but-not-crashed indexer (#218).
pub fn set_heartbeat_timestamp(secs: f64) {
    gauge!(HEARTBEAT_TIMESTAMP).set(secs);
}

pub fn record_events_processed(count: u64) {
    if count > 0 {
        counter!(EVENTS_TOTAL).increment(count);
    }
}

pub fn record_events_skipped(count: u64) {
    if count > 0 {
        counter!(EVENTS_SKIPPED_TOTAL).increment(count);
    }
}

pub fn record_parse_error() {
    counter!(PARSE_ERRORS_TOTAL).increment(1);
}

pub fn record_poll_duration(seconds: f64) {
    histogram!(POLL_DURATION_SECONDS).record(seconds);
}

pub fn record_poll_error() {
    counter!(POLL_ERRORS_TOTAL).increment(1);
}

pub fn record_rpc_retry() {
    counter!(RPC_RETRIES_TOTAL).increment(1);
}

/// Count an RPC call that hit the connect or overall request timeout (issue #214).
pub fn record_rpc_timeout() {
    counter!(RPC_TIMEOUTS_TOTAL).increment(1);
}

/// Publish which endpoint of the configured pool is currently serving traffic
/// (0 = primary), so a silent, sustained failover is visible (issue #213).
pub fn set_rpc_active_endpoint(index: usize) {
    gauge!(RPC_ACTIVE_ENDPOINT).set(index as f64);
}

/// Count a switch to a different RPC endpoint (issue #213).
pub fn record_rpc_failover() {
    counter!(RPC_FAILOVERS_TOTAL).increment(1);
}

/// Publish the number of committed-but-unpublished events. A backlog that keeps
/// climbing means live subscribers are missing data (issue #200).
pub fn set_outbox_backlog(backlog: i64) {
    gauge!(OUTBOX_BACKLOG).set(backlog as f64);
}

/// Count an event delivered to the Redis stream by the relay (issue #200).
pub fn record_outbox_published() {
    counter!(OUTBOX_PUBLISHED_TOTAL).increment(1);
}

/// Count a failed relay publish attempt (issue #200).
pub fn record_outbox_publish_failure() {
    counter!(OUTBOX_PUBLISH_FAILURES_TOTAL).increment(1);
}

/// Increment the per-contract event counter. `contract_id` must be either an
/// allowlisted contract ID or the sentinel `"other"` — never an unbounded value.
pub fn record_events_by_contract(contract_id: &str, count: u64) {
    if count > 0 {
        counter!(EVENTS_BY_CONTRACT_TOTAL, "contract" => contract_id.to_string()).increment(count);
    }
}

pub fn record_decode_duration(seconds: f64) {
    histogram!(EVENT_DECODE_DURATION_SECONDS).record(seconds);
}
