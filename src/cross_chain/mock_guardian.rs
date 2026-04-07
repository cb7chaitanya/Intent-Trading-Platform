//! Mock Wormhole guardian RPC server for local testing.
//!
//! Provides a fully functional HTTP server that mimics the guardian network's
//! `/v1/signed_vaa` and `/v1/signed_vaa_by_tx` endpoints. Generates valid
//! VAA binary payloads that pass `parse_vaa()` and `verify_vaa()`.
//!
//! Features:
//! - Configurable response delay (simulates guardian signing latency)
//! - Configurable failure injection (HTTP errors, malformed VAAs, timeouts)
//! - Pre-registered VAAs keyed by (chain_id, emitter, sequence) or tx_hash
//! - Automatic VAA generation with configurable guardian count
//! - Request logging for test assertions
//!
//! # Usage
//!
//! ```rust,no_run
//! use intent_trading::cross_chain::mock_guardian::MockGuardian;
//!
//! # async fn example() {
//! let mut guardian = MockGuardian::builder()
//!     .num_guardians(13)
//!     .response_delay_ms(100)
//!     .build();
//!
//! // Pre-register a VAA
//! guardian.register_vaa(2, "emitter_hex", 1, b"payload");
//!
//! // Start the server
//! let addr = guardian.start().await;
//!
//! // Use addr as the guardian_rpc URL for WormholeBridge
//! // let bridge = WormholeBridge::new(&format!("http://{addr}"));
//!
//! guardian.shutdown().await;
//! # }
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

// ── VAA builder ─────────────────────────────────────────────

/// Build a valid Wormhole VAA binary payload.
///
/// The resulting bytes pass `WormholeBridge::parse_vaa()` and
/// `WormholeBridge::verify_vaa()` when `num_signatures >= 13`
/// and all guardian indices are unique and < 19.
pub fn build_vaa(
    num_signatures: u8,
    emitter_chain: u16,
    emitter_address: &[u8; 32],
    sequence: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut vaa = Vec::with_capacity(6 + num_signatures as usize * 66 + 51 + payload.len());

    // Header
    vaa.push(1); // version
    vaa.extend_from_slice(&[0, 0, 0, 0]); // guardian_set_index
    vaa.push(num_signatures);

    // Signatures: 66 bytes each (1 byte index + 65 bytes placeholder sig)
    for i in 0..num_signatures {
        vaa.push(i); // guardian index
        vaa.extend_from_slice(&[0u8; 65]); // placeholder secp256k1 sig
    }

    // Body
    vaa.extend_from_slice(&0u32.to_be_bytes()); // timestamp
    vaa.extend_from_slice(&0u32.to_be_bytes()); // nonce
    vaa.extend_from_slice(&emitter_chain.to_be_bytes());
    vaa.extend_from_slice(emitter_address);
    vaa.extend_from_slice(&sequence.to_be_bytes());
    vaa.push(1); // consistency_level

    vaa.extend_from_slice(payload);
    vaa
}

/// Build a VAA with default emitter address (all zeros) for convenience.
pub fn build_vaa_simple(
    num_signatures: u8,
    emitter_chain: u16,
    sequence: u64,
    payload: &[u8],
) -> Vec<u8> {
    build_vaa(num_signatures, emitter_chain, &[0u8; 32], sequence, payload)
}

/// Base64 encode bytes (standard alphabet, with padding).
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}

// ── Failure injection ───────────────────────────────────────

/// Configurable failure mode for the mock server.
#[derive(Debug, Clone)]
pub enum FailureMode {
    /// Return HTTP 500 for the next N requests.
    ServerError { remaining: u32 },
    /// Return HTTP 503 (circuit breaker trigger).
    Unavailable { remaining: u32 },
    /// Return 200 but with malformed JSON.
    MalformedResponse { remaining: u32 },
    /// Return a VAA with fewer than quorum signatures.
    InsufficientQuorum { num_sigs: u8 },
    /// Return a VAA with duplicate guardian indices.
    DuplicateGuardians,
    /// Drop the connection (simulate timeout).
    Timeout { delay: Duration },
}

// ── Server state ────────────────────────────────────────────

/// Key for looking up VAAs by (chain_id, emitter, sequence).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct VaaKey {
    chain_id: u16,
    emitter: String,
    sequence: u64,
}

/// Recorded request for test assertions.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub path: String,
    pub timestamp: std::time::Instant,
}

struct ServerState {
    /// Pre-registered VAAs keyed by (chain_id, emitter, sequence).
    vaas: HashMap<VaaKey, Vec<u8>>,
    /// VAAs keyed by source tx hash (for /v1/signed_vaa_by_tx).
    vaas_by_tx: HashMap<String, Vec<u8>>,
    /// Number of guardian signatures to include in auto-generated VAAs.
    num_guardians: u8,
    /// Delay before responding (simulates guardian signing latency).
    response_delay: Duration,
    /// Active failure injection.
    failure: Option<FailureMode>,
    /// Recorded requests for test assertions.
    requests: Vec<RecordedRequest>,
    /// Total requests served.
    request_count: AtomicU64,
    /// Whether to auto-generate VAAs for unknown keys.
    auto_generate: bool,
    /// Number of requests to return "pending" (null vaaBytes) before
    /// returning the actual VAA. Simulates guardian signing delay.
    pending_count: u32,
    /// Per-key counter of how many times a key was requested.
    key_hits: HashMap<String, u32>,
}

impl ServerState {
    fn new(num_guardians: u8, response_delay: Duration) -> Self {
        Self {
            vaas: HashMap::new(),
            vaas_by_tx: HashMap::new(),
            num_guardians,
            response_delay,
            failure: None,
            requests: Vec::new(),
            request_count: AtomicU64::new(0),
            auto_generate: true,
            pending_count: 0,
            key_hits: HashMap::new(),
        }
    }
}

// ── Public API ──────────────────────────────────────────────

/// Mock Wormhole guardian network for local testing.
pub struct MockGuardian {
    state: Arc<RwLock<ServerState>>,
    cancel: CancellationToken,
    addr: Option<SocketAddr>,
}

impl MockGuardian {
    pub fn builder() -> MockGuardianBuilder {
        MockGuardianBuilder {
            num_guardians: 13,
            response_delay_ms: 0,
            auto_generate: true,
            pending_count: 0,
        }
    }

    /// Register a pre-built VAA for a specific (chain_id, emitter, sequence).
    pub async fn register_vaa(
        &self,
        chain_id: u16,
        emitter: &str,
        sequence: u64,
        payload: &[u8],
    ) {
        let mut state = self.state.write().await;
        let emitter_hex = emitter.strip_prefix("0x").unwrap_or(emitter).to_string();

        // Pad emitter to 32 bytes
        let mut emitter_bytes = [0u8; 32];
        let decoded = hex::decode(&emitter_hex).unwrap_or_default();
        let start = 32 - decoded.len().min(32);
        emitter_bytes[start..].copy_from_slice(&decoded[..decoded.len().min(32)]);

        let vaa = build_vaa(
            state.num_guardians,
            chain_id,
            &emitter_bytes,
            sequence,
            payload,
        );

        state.vaas.insert(
            VaaKey {
                chain_id,
                emitter: emitter_hex,
                sequence,
            },
            vaa,
        );
    }

    /// Register a VAA by source transaction hash.
    pub async fn register_vaa_by_tx(
        &self,
        tx_hash: &str,
        chain_id: u16,
        emitter: &str,
        sequence: u64,
        payload: &[u8],
    ) {
        let mut state = self.state.write().await;
        let emitter_hex = emitter.strip_prefix("0x").unwrap_or(emitter).to_string();

        let mut emitter_bytes = [0u8; 32];
        let decoded = hex::decode(&emitter_hex).unwrap_or_default();
        let start = 32 - decoded.len().min(32);
        emitter_bytes[start..].copy_from_slice(&decoded[..decoded.len().min(32)]);

        let vaa = build_vaa(
            state.num_guardians,
            chain_id,
            &emitter_bytes,
            sequence,
            payload,
        );
        state
            .vaas_by_tx
            .insert(tx_hash.to_string(), vaa);
    }

    /// Inject a failure mode. Affects subsequent requests.
    pub async fn inject_failure(&self, mode: FailureMode) {
        let mut state = self.state.write().await;
        state.failure = Some(mode);
    }

    /// Clear any active failure injection.
    pub async fn clear_failure(&self) {
        let mut state = self.state.write().await;
        state.failure = None;
    }

    /// Get the number of requests served so far.
    pub async fn request_count(&self) -> u64 {
        let state = self.state.read().await;
        state.request_count.load(Ordering::Relaxed)
    }

    /// Get all recorded requests (for test assertions).
    pub async fn recorded_requests(&self) -> Vec<RecordedRequest> {
        let state = self.state.read().await;
        state.requests.clone()
    }

    /// Set the number of "pending" responses before returning actual VAA.
    pub async fn set_pending_count(&self, count: u32) {
        let mut state = self.state.write().await;
        state.pending_count = count;
    }

    /// Get the server address (available after start()).
    pub fn addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// URL string for use as `guardian_rpc`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr.expect("server not started"))
    }

    /// Start the mock server. Returns the bound address.
    pub async fn start(&mut self) -> SocketAddr {
        let state = Arc::clone(&self.state);
        let cancel = self.cancel.clone();

        let app = axum::Router::new()
            .route(
                "/v1/signed_vaa/{chain_id}/{emitter}/{sequence}",
                get(handle_signed_vaa),
            )
            .route(
                "/v1/signed_vaa_by_tx/{tx_hash}",
                get(handle_signed_vaa_by_tx),
            )
            .route("/health", get(|| async { "ok" }))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock guardian");
        let addr = listener.local_addr().unwrap();
        self.addr = Some(addr);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await
                .ok();
        });

        // Wait for server to be ready
        tokio::time::sleep(Duration::from_millis(10)).await;
        addr
    }

    /// Shut down the server.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── Builder ─────────────────────────────────────────────────

pub struct MockGuardianBuilder {
    num_guardians: u8,
    response_delay_ms: u64,
    auto_generate: bool,
    pending_count: u32,
}

impl MockGuardianBuilder {
    /// Number of guardian signatures in generated VAAs (default: 13).
    pub fn num_guardians(mut self, n: u8) -> Self {
        self.num_guardians = n;
        self
    }

    /// Artificial delay before responding (default: 0).
    pub fn response_delay_ms(mut self, ms: u64) -> Self {
        self.response_delay_ms = ms;
        self
    }

    /// Whether to auto-generate VAAs for unknown keys (default: true).
    pub fn auto_generate(mut self, enable: bool) -> Self {
        self.auto_generate = enable;
        self
    }

    /// Number of "pending" (null vaaBytes) responses before returning
    /// the actual VAA (default: 0). Simulates guardian signing delay.
    pub fn pending_count(mut self, n: u32) -> Self {
        self.pending_count = n;
        self
    }

    pub fn build(self) -> MockGuardian {
        let mut state = ServerState::new(
            self.num_guardians,
            Duration::from_millis(self.response_delay_ms),
        );
        state.auto_generate = self.auto_generate;
        state.pending_count = self.pending_count;

        MockGuardian {
            state: Arc::new(RwLock::new(state)),
            cancel: CancellationToken::new(),
            addr: None,
        }
    }
}

// ── HTTP handlers ───────────────────────────────────────────

#[derive(Serialize)]
struct GuardianJsonResponse {
    data: Option<GuardianData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<i32>,
}

#[derive(Serialize)]
struct GuardianData {
    #[serde(rename = "vaaBytes")]
    vaa_bytes: Option<String>,
}

/// GET /v1/signed_vaa/:chain_id/:emitter/:sequence
async fn handle_signed_vaa(
    State(state): State<Arc<RwLock<ServerState>>>,
    Path((chain_id_str, emitter, sequence_str)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let mut st = state.write().await;
    let path = format!("/v1/signed_vaa/{chain_id_str}/{emitter}/{sequence_str}");
    st.requests.push(RecordedRequest {
        path: path.clone(),
        timestamp: std::time::Instant::now(),
    });
    st.request_count.fetch_add(1, Ordering::Relaxed);

    // Apply response delay
    let delay = st.response_delay;
    if delay > Duration::ZERO {
        drop(st);
        tokio::time::sleep(delay).await;
        st = state.write().await;
    }

    // Check failure injection
    if let Some(ref mut failure) = st.failure {
        match failure.clone() {
            FailureMode::ServerError { remaining } => {
                if remaining > 1 {
                    st.failure = Some(FailureMode::ServerError {
                        remaining: remaining - 1,
                    });
                } else {
                    st.failure = None;
                }
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(GuardianJsonResponse {
                        data: None,
                        message: Some("internal error".into()),
                        code: Some(500),
                    }),
                );
            }
            FailureMode::Unavailable { remaining } => {
                if remaining > 1 {
                    st.failure = Some(FailureMode::Unavailable {
                        remaining: remaining - 1,
                    });
                } else {
                    st.failure = None;
                }
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(GuardianJsonResponse {
                        data: None,
                        message: Some("service unavailable".into()),
                        code: Some(503),
                    }),
                );
            }
            FailureMode::MalformedResponse { remaining } => {
                if remaining > 1 {
                    st.failure = Some(FailureMode::MalformedResponse {
                        remaining: remaining - 1,
                    });
                } else {
                    st.failure = None;
                }
                return (
                    StatusCode::OK,
                    Json(GuardianJsonResponse {
                        data: Some(GuardianData {
                            vaa_bytes: Some("!!!not-base64!!!".into()),
                        }),
                        message: None,
                        code: None,
                    }),
                );
            }
            FailureMode::Timeout { delay } => {
                // Hold connection open then drop
                drop(st);
                tokio::time::sleep(delay).await;
                return (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(GuardianJsonResponse {
                        data: None,
                        message: Some("timeout".into()),
                        code: Some(504),
                    }),
                );
            }
            // InsufficientQuorum and DuplicateGuardians are handled below
            // when building the VAA response.
            _ => {}
        }
    }

    let chain_id: u16 = match chain_id_str.parse() {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(GuardianJsonResponse {
                    data: None,
                    message: Some("invalid chain_id".into()),
                    code: Some(400),
                }),
            );
        }
    };
    let sequence: u64 = match sequence_str.parse() {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(GuardianJsonResponse {
                    data: None,
                    message: Some("invalid sequence".into()),
                    code: Some(400),
                }),
            );
        }
    };

    let emitter_clean = emitter.strip_prefix("0x").unwrap_or(&emitter).to_string();

    // Check pending count (simulate signing delay)
    let hit_key = format!("{chain_id}/{emitter_clean}/{sequence}");
    let hits = st.key_hits.entry(hit_key).or_insert(0);
    *hits += 1;
    if *hits <= st.pending_count {
        return (
            StatusCode::OK,
            Json(GuardianJsonResponse {
                data: Some(GuardianData { vaa_bytes: None }),
                message: Some("not yet signed".into()),
                code: None,
            }),
        );
    }

    let key = VaaKey {
        chain_id,
        emitter: emitter_clean.clone(),
        sequence,
    };

    let vaa_bytes = if let Some(vaa) = st.vaas.get(&key) {
        vaa.clone()
    } else if st.auto_generate {
        // Auto-generate a VAA
        let mut emitter_bytes = [0u8; 32];
        let decoded = hex::decode(&emitter_clean).unwrap_or_default();
        let start = 32 - decoded.len().min(32);
        emitter_bytes[start..].copy_from_slice(&decoded[..decoded.len().min(32)]);

        let num_sigs = match &st.failure {
            Some(FailureMode::InsufficientQuorum { num_sigs }) => *num_sigs,
            _ => st.num_guardians,
        };

        let mut vaa = build_vaa(num_sigs, chain_id, &emitter_bytes, sequence, b"auto");

        // Inject duplicate guardian if requested
        if matches!(st.failure, Some(FailureMode::DuplicateGuardians)) {
            if vaa.len() > 6 + 66 {
                // Make second signature use same guardian index as first
                vaa[6 + 66] = vaa[6];
            }
            st.failure = None; // one-shot
        }

        if matches!(st.failure, Some(FailureMode::InsufficientQuorum { .. })) {
            st.failure = None; // one-shot
        }

        vaa
    } else {
        return (
            StatusCode::NOT_FOUND,
            Json(GuardianJsonResponse {
                data: None,
                message: Some("VAA not found".into()),
                code: Some(404),
            }),
        );
    };

    let b64 = base64_encode(&vaa_bytes);

    (
        StatusCode::OK,
        Json(GuardianJsonResponse {
            data: Some(GuardianData {
                vaa_bytes: Some(b64),
            }),
            message: None,
            code: None,
        }),
    )
}

/// GET /v1/signed_vaa_by_tx/:tx_hash
async fn handle_signed_vaa_by_tx(
    State(state): State<Arc<RwLock<ServerState>>>,
    Path(tx_hash): Path<String>,
) -> impl IntoResponse {
    let mut st = state.write().await;
    let path = format!("/v1/signed_vaa_by_tx/{tx_hash}");
    st.requests.push(RecordedRequest {
        path,
        timestamp: std::time::Instant::now(),
    });
    st.request_count.fetch_add(1, Ordering::Relaxed);

    let delay = st.response_delay;
    if delay > Duration::ZERO {
        drop(st);
        tokio::time::sleep(delay).await;
        st = state.write().await;
    }

    // Check failure injection (same as signed_vaa)
    if let Some(ref mut failure) = st.failure {
        match failure.clone() {
            FailureMode::ServerError { remaining } => {
                if remaining > 1 {
                    st.failure = Some(FailureMode::ServerError { remaining: remaining - 1 });
                } else {
                    st.failure = None;
                }
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(GuardianJsonResponse {
                        data: None,
                        message: Some("internal error".into()),
                        code: Some(500),
                    }),
                );
            }
            FailureMode::Unavailable { remaining } => {
                if remaining > 1 {
                    st.failure = Some(FailureMode::Unavailable { remaining: remaining - 1 });
                } else {
                    st.failure = None;
                }
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(GuardianJsonResponse {
                        data: None,
                        message: Some("service unavailable".into()),
                        code: Some(503),
                    }),
                );
            }
            _ => {}
        }
    }

    let vaa = st.vaas_by_tx.get(&tx_hash).cloned();

    match vaa {
        Some(vaa_bytes) => {
            let b64 = base64_encode(&vaa_bytes);
            (
                StatusCode::OK,
                Json(GuardianJsonResponse {
                    data: Some(GuardianData {
                        vaa_bytes: Some(b64),
                    }),
                    message: None,
                    code: None,
                }),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(GuardianJsonResponse {
                data: None,
                message: Some("VAA not found".into()),
                code: Some(404),
            }),
        ),
    }
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_vaa_has_correct_structure() {
        let emitter = [0xABu8; 32];
        let vaa = build_vaa(13, 2, &emitter, 42, b"hello");

        assert_eq!(vaa[0], 1, "version");
        assert_eq!(vaa[5], 13, "num_signatures");

        let body_offset = 6 + 13 * 66;
        let body = &vaa[body_offset..];
        let chain = u16::from_be_bytes([body[8], body[9]]);
        assert_eq!(chain, 2);

        let seq = u64::from_be_bytes(body[42..50].try_into().unwrap());
        assert_eq!(seq, 42);

        let payload = &body[51..];
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn build_vaa_simple_uses_zero_emitter() {
        let vaa = build_vaa_simple(13, 1, 1, b"test");
        let body_offset = 6 + 13 * 66;
        let emitter = &vaa[body_offset + 10..body_offset + 42];
        assert!(emitter.iter().all(|&b| b == 0));
    }

    #[test]
    fn build_vaa_guardian_indices_are_unique() {
        let vaa = build_vaa_simple(19, 2, 1, b"");
        for i in 0..19u8 {
            let idx = vaa[6 + i as usize * 66];
            assert_eq!(idx, i, "guardian index {i} should equal {i}");
        }
    }

    #[test]
    fn build_vaa_passes_minimum_length() {
        let vaa = build_vaa_simple(0, 2, 1, b"");
        // 6 header + 0 sigs + 51 body minimum
        assert!(vaa.len() >= 57);
    }

    #[test]
    fn base64_encode_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_roundtrip_with_vaa() {
        let vaa = build_vaa_simple(13, 2, 99, b"payload");
        let b64 = base64_encode(&vaa);

        // Verify it's valid base64 by checking character set
        assert!(b64.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='));
        assert!(!b64.is_empty());
    }

    #[tokio::test]
    async fn mock_guardian_starts_and_responds() {
        let mut guardian = MockGuardian::builder()
            .num_guardians(13)
            .build();

        let addr = guardian.start().await;
        let url = format!("http://{addr}/health");

        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_returns_auto_generated_vaa() {
        let mut guardian = MockGuardian::builder()
            .num_guardians(15)
            .build();

        guardian.start().await;
        let url = format!("{}/v1/signed_vaa/2/abcd/1", guardian.url());

        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["data"]["vaaBytes"].is_string());

        assert_eq!(guardian.request_count().await, 1);
        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_returns_registered_vaa() {
        let mut guardian = MockGuardian::builder()
            .auto_generate(false)
            .num_guardians(13)
            .build();

        guardian.start().await;
        guardian
            .register_vaa(2, "deadbeef", 42, b"registered_payload")
            .await;

        // Known key returns 200
        let url = format!(
            "{}/v1/signed_vaa/2/deadbeef/42",
            guardian.url()
        );
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["data"]["vaaBytes"].is_string());

        // Unknown key returns 404
        let url_unknown = format!(
            "{}/v1/signed_vaa/2/deadbeef/999",
            guardian.url()
        );
        let resp = reqwest::get(&url_unknown).await.unwrap();
        assert_eq!(resp.status(), 404);

        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_failure_injection_server_error() {
        let mut guardian = MockGuardian::builder().build();
        guardian.start().await;

        guardian
            .inject_failure(FailureMode::ServerError { remaining: 2 })
            .await;

        let url = format!("{}/v1/signed_vaa/2/abc/1", guardian.url());

        // First two requests fail with 500
        let r1 = reqwest::get(&url).await.unwrap();
        assert_eq!(r1.status(), 500);

        let r2 = reqwest::get(&url).await.unwrap();
        assert_eq!(r2.status(), 500);

        // Third request succeeds (failure exhausted)
        let r3 = reqwest::get(&url).await.unwrap();
        assert_eq!(r3.status(), 200);

        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_pending_then_ready() {
        let mut guardian = MockGuardian::builder()
            .pending_count(2)
            .build();

        guardian.start().await;
        let url = format!("{}/v1/signed_vaa/2/abc/1", guardian.url());

        // First 2 requests return pending (null vaaBytes)
        let r1 = reqwest::get(&url).await.unwrap();
        let b1: serde_json::Value = r1.json().await.unwrap();
        assert!(b1["data"]["vaaBytes"].is_null(), "first request should be pending");

        let r2 = reqwest::get(&url).await.unwrap();
        let b2: serde_json::Value = r2.json().await.unwrap();
        assert!(b2["data"]["vaaBytes"].is_null(), "second request should be pending");

        // Third request returns actual VAA
        let r3 = reqwest::get(&url).await.unwrap();
        let b3: serde_json::Value = r3.json().await.unwrap();
        assert!(b3["data"]["vaaBytes"].is_string(), "third request should have VAA");

        assert_eq!(guardian.request_count().await, 3);
        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_by_tx_endpoint() {
        let mut guardian = MockGuardian::builder()
            .num_guardians(13)
            .build();

        guardian.start().await;
        guardian
            .register_vaa_by_tx("0xabc123", 2, "deadbeef", 1, b"by_tx")
            .await;

        let url = format!("{}/v1/signed_vaa_by_tx/0xabc123", guardian.url());
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["data"]["vaaBytes"].is_string());

        // Unknown tx returns 404
        let url2 = format!("{}/v1/signed_vaa_by_tx/0xunknown", guardian.url());
        let resp2 = reqwest::get(&url2).await.unwrap();
        assert_eq!(resp2.status(), 404);

        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_records_requests() {
        let mut guardian = MockGuardian::builder().build();
        guardian.start().await;

        let url = format!("{}/v1/signed_vaa/2/abc/1", guardian.url());
        reqwest::get(&url).await.unwrap();
        reqwest::get(&url).await.unwrap();

        let reqs = guardian.recorded_requests().await;
        assert_eq!(reqs.len(), 2);
        assert!(reqs[0].path.contains("/v1/signed_vaa/2/abc/1"));

        guardian.shutdown().await;
    }

    #[tokio::test]
    async fn mock_guardian_response_delay() {
        let mut guardian = MockGuardian::builder()
            .response_delay_ms(100)
            .build();

        guardian.start().await;
        let url = format!("{}/v1/signed_vaa/2/abc/1", guardian.url());

        let start = std::time::Instant::now();
        reqwest::get(&url).await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(80),
            "Response should be delayed: {elapsed:?}"
        );

        guardian.shutdown().await;
    }
}
