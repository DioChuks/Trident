use std::time::Duration;

use serde::{Deserialize, Serialize};
use trident_common::TridentError;

use crate::metrics;

/// A single raw event as returned by the Stellar RPC `getEvents` method.
/// Topics and data are base64-encoded XDR strings; the parser decodes them.
#[derive(Debug, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    /// Ledger sequence number as a numeric string.
    pub ledger: String,
    #[serde(rename = "ledgerClosedAt")]
    pub ledger_closed_at: String,
    #[serde(rename = "contractId")]
    pub contract_id: Option<String>,
    pub id: String,
    #[serde(rename = "pagingToken")]
    pub paging_token: String,
    #[serde(rename = "txHash")]
    pub tx_hash: String,
    /// Ordered list of base64 XDR-encoded ScVal topic values.
    pub topic: Vec<String>,
    /// Base64 XDR-encoded ScVal event body.
    pub value: String,
    #[serde(rename = "inSuccessfulContractCall")]
    pub in_successful_contract_call: bool,
}

#[derive(Debug)]
pub struct EventsPage {
    pub events: Vec<RawEvent>,
    pub latest_ledger: u64,
}

// ---------------------------------------------------------------------------
// JSON-RPC wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest<'a, P: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
struct JsonRpcResponse<R> {
    result: Option<R>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Serialize)]
struct GetLedgersParams {
    #[serde(rename = "startLedger")]
    start_ledger: u64,
    pagination: LedgerPagination,
}

#[derive(Serialize)]
struct LedgerPagination {
    limit: u32,
}

#[derive(Deserialize)]
struct GetLedgersResult {
    ledgers: Vec<LedgerSummary>,
}

#[derive(Deserialize)]
struct LedgerSummary {
    hash: String,
}

#[derive(Serialize)]
struct GetEventsParams {
    #[serde(rename = "startLedger", skip_serializing_if = "Option::is_none")]
    start_ledger: Option<u64>,
    filters: Vec<serde_json::Value>,
    pagination: Pagination,
}

#[derive(Serialize)]
struct Pagination {
    limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct GetEventsResult {
    events: Vec<RawEvent>,
    #[serde(rename = "latestLedger")]
    latest_ledger: u64,
}

// ---------------------------------------------------------------------------
// RPC client
// ---------------------------------------------------------------------------

/// HTTP transport settings for the RPC client (issue #214).
///
/// A default `reqwest::Client` has no request timeout at all, so a stalled
/// response blocks the poll loop forever — the retry wrapper only sees returned
/// errors, never a call that never returns. Every field here is derived from
/// `Config` so operators can tune it per environment.
#[derive(Debug, Clone)]
pub struct RpcHttpSettings {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub tcp_keepalive: Duration,
}

impl Default for RpcHttpSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 8,
            tcp_keepalive: Duration::from_secs(60),
        }
    }
}

impl RpcHttpSettings {
    /// Build the shared `reqwest::Client`: bounded connect/request timeouts plus
    /// keep-alive and idle-pool tuning so successive polls reuse connections
    /// instead of paying a fresh TCP + TLS handshake each time.
    fn build_client(&self) -> Result<reqwest::Client, TridentError> {
        reqwest::Client::builder()
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .pool_idle_timeout(self.pool_idle_timeout)
            .pool_max_idle_per_host(self.pool_max_idle_per_host)
            .tcp_keepalive(self.tcp_keepalive)
            .build()
            .map_err(|e| {
                TridentError::config(
                    anyhow::Error::new(e).context("failed to build RPC HTTP client"),
                )
            })
    }
}

/// Convert a `reqwest` transport failure into a retryable [`TridentError`],
/// tagging timeouts explicitly so they are visible in logs and metrics.
///
/// `RpcError` is already classified `Severity::Retryable`, which is what makes
/// the backoff wrapper and the poll loop treat a timeout as a transient failure
/// rather than a poison input (issue #214).
fn rpc_transport_error(err: reqwest::Error, context: &'static str) -> TridentError {
    if err.is_timeout() {
        metrics::record_rpc_timeout();
        return TridentError::rpc(anyhow::Error::new(err).context(format!("{context} timed out")));
    }
    TridentError::rpc(anyhow::Error::new(err).context(context))
}

pub struct RpcClient {
    http: reqwest::Client,
    url: String,
}

impl RpcClient {
    /// Build a client with the default transport settings. Prefer
    /// [`RpcClient::with_settings`] in production so the configured timeouts
    /// apply.
    pub fn new(url: String) -> Self {
        Self::with_settings(url, &RpcHttpSettings::default())
            .expect("default RPC HTTP settings must build a client")
    }

    /// Build a client whose transport honours the configured timeouts and
    /// connection-pool settings (issue #214).
    pub fn with_settings(url: String, settings: &RpcHttpSettings) -> Result<Self, TridentError> {
        Ok(Self {
            http: settings.build_client()?,
            url,
        })
    }

    /// Fetch the ledger hash for a given sequence number via `getLedgers`.
    /// Returns `None` if the RPC does not know about that ledger yet.
    pub async fn get_ledger(&self, sequence: u64) -> Result<Option<String>, TridentError> {
        let params = GetLedgersParams {
            start_ledger: sequence,
            pagination: LedgerPagination { limit: 1 },
        };
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 2,
            method: "getLedgers",
            params,
        };

        let resp = self
            .http
            .post(&self.url)
            .json(&req)
            .send()
            .await
            .map_err(|e| rpc_transport_error(e, "getLedgers HTTP failed"))?;

        let body: JsonRpcResponse<GetLedgersResult> = resp
            .json()
            .await
            .map_err(|e| rpc_transport_error(e, "getLedgers decode failed"))?;

        if let Some(err) = body.error {
            return Err(TridentError::rpc(anyhow::anyhow!(
                "getLedgers RPC error {}: {}",
                err.code,
                err.message
            )));
        }

        let hash = body
            .result
            .and_then(|r| r.ledgers.into_iter().next())
            .map(|l| l.hash);

        Ok(hash)
    }

    /// Fetch a page of events from the Stellar RPC node.
    ///
    /// Pass `start_ledger` on the first call to anchor the scan position.
    /// On subsequent calls pass `cursor` (the `paging_token` from the last
    /// event received) to continue pagination. Only one of the two should be
    /// set at a time — the RPC rejects requests that supply both.
    ///
    /// `limit` controls the page size; callers should pass `config.max_events_per_poll`.
    pub async fn get_events(
        &self,
        start_ledger: Option<u64>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<EventsPage, TridentError> {
        let params = GetEventsParams {
            start_ledger,
            filters: vec![],
            pagination: Pagination { limit, cursor },
        };

        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getEvents",
            params,
        };

        let resp = self
            .http
            .post(&self.url)
            .json(&req)
            .send()
            .await
            .map_err(|e| rpc_transport_error(e, "HTTP request failed"))?;

        let body: JsonRpcResponse<GetEventsResult> = resp
            .json()
            .await
            .map_err(|e| rpc_transport_error(e, "Failed to decode RPC response"))?;

        if let Some(err) = body.error {
            return Err(TridentError::rpc(anyhow::anyhow!(
                "RPC error {}: {}",
                err.code,
                err.message
            )));
        }

        let result = body
            .result
            .ok_or_else(|| TridentError::rpc(anyhow::anyhow!("Empty result in RPC response")))?;

        Ok(EventsPage {
            events: result.events,
            latest_ledger: result.latest_ledger,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use trident_common::Severity;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast_timeout_settings() -> RpcHttpSettings {
        RpcHttpSettings {
            connect_timeout: Duration::from_millis(300),
            request_timeout: Duration::from_millis(300),
            ..RpcHttpSettings::default()
        }
    }

    /// A deliberately slow endpoint must abort within the configured request
    /// timeout instead of hanging the caller (issue #214).
    #[tokio::test]
    async fn slow_endpoint_aborts_within_request_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_string("{}"),
            )
            .mount(&server)
            .await;

        let client = RpcClient::with_settings(server.uri(), &fast_timeout_settings()).unwrap();

        let started = Instant::now();
        let err = client
            .get_events(Some(1), None, 10)
            .await
            .expect_err("slow endpoint must not succeed");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(5),
            "call should abort at the timeout, took {elapsed:?}"
        );
        assert!(
            err.to_string().contains("timed out"),
            "timeout should be reported as such, got: {err}"
        );
    }

    /// A timeout must stay classified as retryable so the backoff wrapper and
    /// the circuit breaker engage rather than the poll cycle being skipped.
    #[tokio::test]
    async fn timeout_is_classified_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_string("{}"),
            )
            .mount(&server)
            .await;

        let client = RpcClient::with_settings(server.uri(), &fast_timeout_settings()).unwrap();
        let err = client.get_ledger(42).await.expect_err("must time out");

        assert_eq!(err.severity(), Severity::Retryable);
        assert!(err.retryable());
    }

    /// The settings are applied to a real client build — a bad combination is a
    /// config error surfaced at startup, not a silent default.
    #[test]
    fn settings_build_a_client() {
        assert!(RpcHttpSettings::default().build_client().is_ok());
    }
}
