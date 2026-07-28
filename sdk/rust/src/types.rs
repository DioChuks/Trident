use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::retry::RetryConfig;

/// Stellar network selection.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    #[default]
    Testnet,
    Futurenet,
}

impl Network {
    pub fn as_str(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Futurenet => "futurenet",
        }
    }
}

/// Configuration for [`TridentClient`](crate::TridentClient).
#[derive(Debug, Clone)]
pub struct TridentConfig {
    /// Base URL of the Trident REST API.
    pub api_url: String,
    /// API key sent as `X-API-Key` on every request.
    pub api_key: String,
    /// Target Stellar network.
    pub network: Network,
    /// Per-request timeout. Defaults to 30 seconds.
    pub timeout: Duration,
    /// Retry policy applied to idempotent (GET) requests, honouring
    /// `Retry-After` on 429/503 responses. `None` disables retries — the
    /// default. Overridden per-call by the `*_with_retry` client methods.
    pub retry: Option<RetryConfig>,
}

impl Default for TridentConfig {
    fn default() -> Self {
        TridentConfig {
            api_url: "https://trident-api.fly.dev".to_string(),
            api_key: String::new(),
            network: Network::Testnet,
            timeout: Duration::from_secs(30),
            retry: None,
        }
    }
}

/// Parameters for [`query_events`](crate::TridentClient::query_events).
#[derive(Debug, Default, Clone)]
pub struct QueryParams {
    pub contract_id: Option<String>,
    pub topic_0: Option<String>,
    pub topic_1: Option<String>,
    pub from_ledger: Option<u64>,
    pub to_ledger: Option<u64>,
    /// Pagination cursor returned by a previous call.
    pub after: Option<String>,
    /// Maximum number of events to return (default: 50).
    pub first: Option<u32>,
    pub event_type: Option<String>,
}

/// Category of a Soroban event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Contract,
    System,
    Diagnostic,
}

/// A single Soroban event returned by the Trident API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanEvent {
    pub id: String,
    pub contract_id: String,
    pub ledger_sequence: u64,
    pub ledger_timestamp: String,
    pub transaction_hash: String,
    pub event_index: u32,
    pub event_type: EventType,
    pub topics: Vec<String>,
    /// Decoded event body. Scalar XDR types are JSON primitives; maps/vecs are
    /// JSON objects/arrays.
    pub data: serde_json::Value,
    pub created_at: String,
}

/// A page of events returned by [`query_events`](crate::TridentClient::query_events).
#[derive(Debug)]
pub struct PaginatedEvents {
    pub events: Vec<SorobanEvent>,
    /// Pass as `after` in the next call to get the next page. `None` when no
    /// more pages exist.
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ContractStatsQuery {
    pub from_ledger: Option<u64>,
    pub to_ledger: Option<u64>,
    pub network: Option<Network>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthChecks {
    pub postgres: String,
    pub redis: String,
    #[serde(rename = "grpc_api")]
    pub grpc_api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub indexer_lag: Option<i64>,
    pub checks: HealthChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerStatsResponse {
    pub last_ledger_indexed: Option<i64>,
    pub chain_tip_ledger: Option<i64>,
    pub lag_ledgers: Option<i64>,
    pub events_indexed_total: Option<i64>,
    pub events_last_poll: Option<i64>,
    pub avg_poll_duration_ms: Option<i64>,
    pub last_poll_at: Option<String>,
    pub status: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractStats {
    pub contract_id: String,
    pub event_count: i64,
    pub last_seen_ledger: i64,
    pub last_seen_at: String,
    pub invocation_count: Option<i64>,
    pub total_fee_charged: Option<i64>,
    pub avg_fee_charged: Option<f64>,
    pub avg_cpu_instructions: Option<f64>,
    pub avg_read_bytes: Option<f64>,
    pub avg_write_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractStatsResponse {
    pub contracts: Vec<ContractStats>,
    pub from_ledger: i64,
    pub to_ledger: i64,
    pub network: String,
    pub generated_at: String,
}
