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
pub const RPC_REQUEST_DURATION_SECONDS: &str = "trident_indexer_rpc_request_duration_seconds";
pub const RPC_ERRORS_TOTAL: &str = "trident_indexer_rpc_errors_total";
pub const HEARTBEAT_TIMESTAMP_SECONDS: &str = "trident_indexer_heartbeat_timestamp_seconds";
pub const DB_POOL_SIZE: &str = "trident_indexer_db_pool_size";
pub const DB_POOL_IDLE_CONNECTIONS: &str = "trident_indexer_db_pool_idle_connections";

/// Install the global Prometheus recorder and start serving `/metrics` on
/// `port`. Must be called once, before the streamer starts recording.
pub fn install(port: u16) -> Result<(), TridentError> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();

    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
        .map_err(|e| TridentError::ConfigError(format!("failed to start metrics exporter: {e}")))?;

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
    describe_histogram!(
        RPC_REQUEST_DURATION_SECONDS,
        "Stellar RPC call latency in seconds, labeled by method"
    );
    describe_counter!(
        RPC_ERRORS_TOTAL,
        "Total Stellar RPC calls that returned an error, labeled by method"
    );
    describe_gauge!(
        HEARTBEAT_TIMESTAMP_SECONDS,
        "Unix timestamp of the last completed poll-loop iteration (dead-man's-switch)"
    );
    describe_gauge!(
        DB_POOL_SIZE,
        "Current number of connections in the indexer's Postgres pool"
    );
    describe_gauge!(
        DB_POOL_IDLE_CONNECTIONS,
        "Current number of idle connections in the indexer's Postgres pool"
    );

    // Counters only render in the scrape output once touched at least once;
    // seed them at zero so /metrics is complete from the very first scrape.
    counter!(EVENTS_TOTAL).increment(0);
    counter!(EVENTS_SKIPPED_TOTAL).increment(0);
    counter!(PARSE_ERRORS_TOTAL).increment(0);
    counter!(POLL_ERRORS_TOTAL).increment(0);
    counter!(RPC_RETRIES_TOTAL).increment(0);
    gauge!(LEDGER_LAG).set(0.0);
    record_heartbeat();

    tracing::info!(port, "Metrics endpoint listening");
    Ok(())
}

pub fn set_ledger_lag(lag: i64) {
    gauge!(LEDGER_LAG).set(lag as f64);
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

/// Record the latency and outcome of a single Stellar RPC call.
/// `method` is a low-cardinality label (e.g. "getEvents", "getLedgers").
pub fn record_rpc_call(method: &'static str, duration_secs: f64, is_error: bool) {
    histogram!(RPC_REQUEST_DURATION_SECONDS, "method" => method).record(duration_secs);
    if is_error {
        counter!(RPC_ERRORS_TOTAL, "method" => method).increment(1);
    }
}

/// Update the dead-man's-switch heartbeat gauge to the current unix time.
/// Called once per poll-loop iteration regardless of outcome — Prometheus
/// alerts on this going stale, which flags a hung or crashed indexer even
/// when lag itself looks fine.
pub fn record_heartbeat() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    gauge!(HEARTBEAT_TIMESTAMP_SECONDS).set(now);
}

/// Record the indexer's own Postgres pool utilisation.
pub fn set_db_pool_stats(size: u32, idle: u32) {
    gauge!(DB_POOL_SIZE).set(size as f64);
    gauge!(DB_POOL_IDLE_CONNECTIONS).set(idle as f64);
}
