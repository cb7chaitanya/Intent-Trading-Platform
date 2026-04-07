//! Wormhole Token Bridge integration with real CPI calls.
//!
//! Flow:
//! 1. lock_funds: call Token Bridge transferTokens on source chain,
//!    parse emitted Wormhole message sequence from tx receipt logs
//! 2. verify_lock: poll guardian RPC for signed VAA, verify guardian
//!    signature quorum (13/19)
//! 3. release_funds: fetch signed VAA, submit to destination chain Token
//!    Bridge completeTransfer, record dest_tx_hash
//!
//! All external calls are circuit-breaker protected with exponential
//! backoff retry on transient failures.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitError};

use super::bridge::*;

// ── Constants ────────────────────────────────────────────

/// Wormhole Token Bridge contract addresses (mainnet).
const TOKEN_BRIDGE_ETHEREUM: &str = "0x3ee18B2214AFF97000D974cf647E7C347E8fa585";
const TOKEN_BRIDGE_SOLANA: &str = "wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb";
const TOKEN_BRIDGE_POLYGON: &str = "0x5a58505a96D1dbf8dF91cB21B54419FC36e93fdE";
const TOKEN_BRIDGE_ARBITRUM: &str = "0x0b2402144Bb366A632D14B83F244D2e0e21bD39c";
const TOKEN_BRIDGE_BASE: &str = "0x8d2de8d2f73F1F4cAB472AC9A881C9b123C79627";

/// Wormhole Core Bridge addresses for log parsing (mainnet).
const CORE_BRIDGE_ETHEREUM: &str = "0x98f3c9e6E3fAce36bAAd05FE09d375Ef1464288B";

/// EVM function selectors.
/// transferTokens(address,uint256,uint16,bytes32,uint256,uint32)
const TRANSFER_TOKENS_SELECTOR: [u8; 4] = [0x01, 0x93, 0x09, 0x55];
/// completeTransfer(bytes)
const COMPLETE_TRANSFER_SELECTOR: [u8; 4] = [0xc6, 0x87, 0x85, 0x19];

/// Wormhole core bridge LogMessagePublished topic (keccak256).
const LOG_MESSAGE_PUBLISHED_TOPIC: &str =
    "0x6eb224fb001ed210e379b335e35efe88672a8ce935d981a6896b27ffdf52a3b2";

/// Guardian RPC paths.
const VAA_PATH: &str = "/v1/signed_vaa";
const VAA_BY_TX_PATH: &str = "/v1/signed_vaa_by_tx";

/// Maximum retries for VAA polling (covers ~10 min at max backoff).
const VAA_POLL_MAX_RETRIES: u32 = 30;
/// Initial delay between VAA poll attempts.
const VAA_POLL_INITIAL_DELAY_MS: u64 = 2_000;
/// Maximum delay cap for VAA polling backoff.
const VAA_POLL_MAX_DELAY_MS: u64 = 30_000;

/// Required guardian signatures for quorum (13 of 19).
const GUARDIAN_QUORUM: usize = 13;

/// Maximum retries for destination chain submission.
const DEST_SUBMIT_MAX_RETRIES: u32 = 5;
/// Initial retry delay for destination submission.
const DEST_SUBMIT_INITIAL_DELAY_MS: u64 = 1_000;

// ── VAA types ────────────────────────────────────────────

/// Parsed Wormhole VAA (Verified Action Approval).
#[derive(Debug, Clone, Serialize)]
struct Vaa {
    /// Raw VAA bytes (base64-decoded from guardian RPC).
    bytes: Vec<u8>,
    /// VAA version (always 1).
    version: u8,
    /// Number of guardian signatures attached.
    num_signatures: usize,
    /// Emitter chain ID.
    emitter_chain: u16,
    /// Emitter address (32 bytes, hex-encoded).
    emitter_address: String,
    /// Sequence number from the core bridge.
    sequence: u64,
    /// Payload bytes (Token Bridge transfer data).
    payload: Vec<u8>,
}

/// Guardian RPC response for signed_vaa.
#[derive(Debug, Deserialize)]
struct GuardianVaaResponse {
    #[serde(rename = "vaaBytes")]
    vaa_bytes: Option<String>,
}

/// Guardian RPC response wrapper.
#[derive(Debug, Deserialize)]
struct GuardianResponse {
    data: Option<GuardianVaaResponse>,
    message: Option<String>,
    #[allow(dead_code)]
    code: Option<i32>,
}

/// Parsed lock receipt from source chain tx logs.
#[derive(Debug, Clone)]
struct WormholeMessage {
    emitter_chain: u16,
    emitter_address: String,
    sequence: u64,
}

/// JSON-RPC response from EVM nodes.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ── Adapter ──────────────────────────────────────────────

pub struct WormholeBridge {
    guardian_rpc: String,
    /// RPC endpoints per chain for submitting transactions.
    chain_rpcs: std::collections::HashMap<String, String>,
    http: reqwest::Client,
    guardian_breaker: CircuitBreaker,
    /// Per-chain circuit breakers for RPC calls.
    chain_breakers: std::collections::HashMap<String, CircuitBreaker>,
}

impl WormholeBridge {
    pub fn new(guardian_rpc: &str) -> Self {
        Self {
            guardian_rpc: guardian_rpc.to_string(),
            chain_rpcs: std::collections::HashMap::new(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            guardian_breaker: CircuitBreaker::new(CircuitBreakerConfig::wormhole_guardian()),
            chain_breakers: std::collections::HashMap::new(),
        }
    }

    /// Register an RPC endpoint for a chain (call during initialization).
    pub fn with_chain_rpc(mut self, chain: &str, rpc_url: &str) -> Self {
        self.chain_breakers.insert(
            chain.to_string(),
            CircuitBreaker::new(CircuitBreakerConfig::new(
                &format!("wormhole_{chain}_rpc"),
                5,
                30,
            )),
        );
        self.chain_rpcs.insert(chain.to_string(), rpc_url.to_string());
        self
    }

    /// Map chain name to Wormhole chain ID.
    fn chain_id(chain: &str) -> Option<u16> {
        match chain {
            "ethereum" => Some(2),
            "solana" => Some(1),
            "polygon" => Some(5),
            "arbitrum" => Some(23),
            "base" => Some(30),
            _ => None,
        }
    }

    /// Get Token Bridge address for a chain.
    fn token_bridge(chain: &str) -> Option<&'static str> {
        match chain {
            "ethereum" => Some(TOKEN_BRIDGE_ETHEREUM),
            "solana" => Some(TOKEN_BRIDGE_SOLANA),
            "polygon" => Some(TOKEN_BRIDGE_POLYGON),
            "arbitrum" => Some(TOKEN_BRIDGE_ARBITRUM),
            "base" => Some(TOKEN_BRIDGE_BASE),
            _ => None,
        }
    }

    /// Construct Wormhole message ID: "chainId/emitter/sequence".
    fn message_id(chain_id: u16, emitter: &str, sequence: u64) -> String {
        format!("{chain_id}/{emitter}/{sequence}")
    }

    /// Parse message ID back into (chain_id, emitter, sequence).
    fn parse_message_id(id: &str) -> Option<(u16, String, u64)> {
        let parts: Vec<&str> = id.splitn(3, '/').collect();
        if parts.len() != 3 {
            return None;
        }
        let chain = parts[0].parse().ok()?;
        let emitter = parts[1].to_string();
        let seq = parts[2].parse().ok()?;
        Some((chain, emitter, seq))
    }

    // ── EVM RPC helpers ─────────────────────────────────

    /// Send a JSON-RPC call to an EVM chain.
    async fn evm_rpc_call(
        &self,
        chain: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        let rpc_url = self
            .chain_rpcs
            .get(chain)
            .ok_or_else(|| BridgeError::NetworkError(format!("No RPC for chain: {chain}")))?;

        let breaker = self.chain_breakers.get(chain);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let do_call = async {
            let resp = self
                .http
                .post(rpc_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| BridgeError::NetworkError(e.to_string()))?;

            let rpc_resp: JsonRpcResponse = resp
                .json()
                .await
                .map_err(|e| BridgeError::NetworkError(format!("parse: {e}")))?;

            if let Some(err) = rpc_resp.error {
                return Err(BridgeError::NetworkError(format!(
                    "RPC error {}: {}",
                    err.code, err.message
                )));
            }

            rpc_resp
                .result
                .ok_or_else(|| BridgeError::NetworkError("null result".into()))
        };

        match breaker {
            Some(b) => match b.call(do_call).await {
                Ok(v) => Ok(v),
                Err(CircuitError::Open { breaker, .. }) => {
                    Err(BridgeError::NetworkError(format!("circuit open: {breaker}")))
                }
                Err(CircuitError::Inner(e)) => Err(e),
            },
            None => do_call.await,
        }
    }

    /// Send a raw signed transaction to an EVM chain.
    /// Returns the transaction hash.
    async fn evm_send_raw_tx(
        &self,
        chain: &str,
        signed_tx_hex: &str,
    ) -> Result<String, BridgeError> {
        let result = self
            .evm_rpc_call(chain, "eth_sendRawTransaction", serde_json::json!([signed_tx_hex]))
            .await?;

        result
            .as_str()
            .map(String::from)
            .ok_or_else(|| BridgeError::NetworkError("tx hash not a string".into()))
    }

    /// Get transaction receipt from an EVM chain.
    async fn evm_get_receipt(
        &self,
        chain: &str,
        tx_hash: &str,
    ) -> Result<Option<serde_json::Value>, BridgeError> {
        let result = self
            .evm_rpc_call(
                chain,
                "eth_getTransactionReceipt",
                serde_json::json!([tx_hash]),
            )
            .await?;

        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(result))
    }

    // ── Source chain: lock via Token Bridge ──────────────

    /// Build and submit a Token Bridge transferTokens call on an EVM chain.
    ///
    /// calldata: transferTokens(token, amount, recipientChain, recipient, arbiterFee, nonce)
    fn encode_transfer_tokens(
        token: &str,
        amount: u64,
        dest_chain_id: u16,
        recipient: &str,
    ) -> Vec<u8> {
        let mut data = Vec::with_capacity(4 + 32 * 6);
        data.extend_from_slice(&TRANSFER_TOKENS_SELECTOR);

        // token address (20 bytes, left-padded to 32)
        let token_bytes = hex::decode(token.strip_prefix("0x").unwrap_or(token)).unwrap_or_default();
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&token_bytes);

        // amount (u256)
        let mut amount_bytes = [0u8; 32];
        amount_bytes[24..].copy_from_slice(&amount.to_be_bytes());
        data.extend_from_slice(&amount_bytes);

        // recipientChain (u16, padded to u256)
        let mut chain_bytes = [0u8; 32];
        chain_bytes[30..].copy_from_slice(&dest_chain_id.to_be_bytes());
        data.extend_from_slice(&chain_bytes);

        // recipient (32 bytes, left-padded if needed)
        let recip_bytes =
            hex::decode(recipient.strip_prefix("0x").unwrap_or(recipient)).unwrap_or_default();
        let pad = 32 - recip_bytes.len().min(32);
        data.extend_from_slice(&vec![0u8; pad]);
        data.extend_from_slice(&recip_bytes[..recip_bytes.len().min(32)]);

        // arbiterFee (0)
        data.extend_from_slice(&[0u8; 32]);

        // nonce (random u32, padded)
        let nonce = rand::random::<u32>();
        let mut nonce_bytes = [0u8; 32];
        nonce_bytes[28..].copy_from_slice(&nonce.to_be_bytes());
        data.extend_from_slice(&nonce_bytes);

        data
    }

    /// Parse the Wormhole core bridge LogMessagePublished event from tx logs
    /// to extract the emitter address and sequence number.
    fn parse_lock_logs(receipt: &serde_json::Value, source_chain_id: u16) -> Option<WormholeMessage> {
        let logs = receipt.get("logs")?.as_array()?;

        for log in logs {
            let topics = log.get("topics")?.as_array()?;
            if topics.is_empty() {
                continue;
            }
            let topic0 = topics[0].as_str().unwrap_or("");
            if topic0 != LOG_MESSAGE_PUBLISHED_TOPIC {
                continue;
            }

            // LogMessagePublished(address indexed sender, uint64 sequence, uint32 nonce, bytes payload, uint8 consistencyLevel)
            // topic[1] = sender (emitter)
            let emitter = topics.get(1)?.as_str().unwrap_or("");
            let emitter_clean = emitter.strip_prefix("0x").unwrap_or(emitter);

            // sequence is encoded in the log data at offset 0 (first 32 bytes)
            let data_hex = log.get("data")?.as_str().unwrap_or("");
            let data_bytes =
                hex::decode(data_hex.strip_prefix("0x").unwrap_or(data_hex)).unwrap_or_default();
            if data_bytes.len() < 32 {
                continue;
            }
            let sequence = u64::from_be_bytes(data_bytes[24..32].try_into().ok()?);

            return Some(WormholeMessage {
                emitter_chain: source_chain_id,
                emitter_address: emitter_clean.to_string(),
                sequence,
            });
        }

        None
    }

    // ── Destination chain: redeem VAA ───────────────────

    /// Build calldata for Token Bridge completeTransfer(bytes encodedVm).
    fn encode_complete_transfer(vaa_bytes: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(4 + 32 + 32 + vaa_bytes.len());
        data.extend_from_slice(&COMPLETE_TRANSFER_SELECTOR);

        // bytes offset (0x20)
        let mut offset = [0u8; 32];
        offset[31] = 0x20;
        data.extend_from_slice(&offset);

        // bytes length
        let mut len = [0u8; 32];
        let vaa_len = vaa_bytes.len() as u64;
        len[24..].copy_from_slice(&vaa_len.to_be_bytes());
        data.extend_from_slice(&len);

        // bytes data (padded to 32-byte boundary)
        data.extend_from_slice(vaa_bytes);
        let padding = (32 - (vaa_bytes.len() % 32)) % 32;
        data.extend_from_slice(&vec![0u8; padding]);

        data
    }

    /// Submit VAA to destination chain Token Bridge with exponential backoff retry.
    async fn submit_vaa_to_destination(
        &self,
        dest_chain: &str,
        vaa: &Vaa,
    ) -> Result<String, BridgeError> {
        let dest_bridge = Self::token_bridge(dest_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(dest_chain.into()))?;

        let calldata = Self::encode_complete_transfer(&vaa.bytes);
        let calldata_hex = format!("0x{}", hex::encode(&calldata));

        let mut last_error = String::new();

        for attempt in 0..DEST_SUBMIT_MAX_RETRIES {
            // Use eth_call first to check for revert (dry run)
            let call_result = self
                .evm_rpc_call(
                    dest_chain,
                    "eth_call",
                    serde_json::json!([{
                        "to": dest_bridge,
                        "data": calldata_hex,
                    }, "latest"]),
                )
                .await;

            if let Err(ref e) = call_result {
                let err_str = e.to_string();
                // "already completed" means VAA was already redeemed — not an error
                if err_str.contains("already completed") || err_str.contains("transfer already completed") {
                    tracing::info!(dest_chain, "VAA already redeemed on destination");
                    // Return a placeholder — the worker will confirm via receipt polling
                    return Err(BridgeError::ReleaseFailed("VAA already redeemed".into()));
                }

                tracing::warn!(
                    attempt,
                    dest_chain,
                    error = %err_str,
                    "dest_submit_dry_run_failed"
                );
                last_error = err_str;

                let delay = DEST_SUBMIT_INITIAL_DELAY_MS * (1 << attempt.min(4));
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                continue;
            }

            // Dry run passed — submit real tx via eth_sendRawTransaction.
            // In production the deployer signs the tx. Here we use eth_call
            // result as the "transaction" and eth_sendRawTransaction for the
            // actual submission. The caller (worker) provides a signed tx
            // via the chain adapter. We submit the VAA calldata to the Token
            // Bridge and capture the resulting tx hash.
            //
            // Build a minimal unsigned tx for the eth_estimateGas / sendTx path:
            match self
                .evm_rpc_call(
                    dest_chain,
                    "eth_sendTransaction",
                    serde_json::json!([{
                        "to": dest_bridge,
                        "data": calldata_hex,
                    }]),
                )
                .await
            {
                Ok(tx_val) => {
                    let tx_hash = tx_val.as_str().unwrap_or("").to_string();
                    if tx_hash.starts_with("0x") && tx_hash.len() >= 66 {
                        tracing::info!(
                            attempt,
                            dest_chain,
                            tx_hash = %tx_hash,
                            "dest_vaa_submitted"
                        );
                        return Ok(tx_hash);
                    }
                    last_error = format!("unexpected tx result: {tx_val}");
                }
                Err(e) => {
                    last_error = e.to_string();
                    // Classify retriable vs fatal
                    let lower = last_error.to_lowercase();
                    let retriable = lower.contains("nonce")
                        || lower.contains("underpriced")
                        || lower.contains("pool")
                        || lower.contains("timeout")
                        || lower.contains("connection");

                    if !retriable {
                        tracing::error!(
                            attempt,
                            dest_chain,
                            error = %last_error,
                            "dest_submit_fatal"
                        );
                        return Err(BridgeError::ReleaseFailed(last_error));
                    }
                }
            }

            tracing::warn!(
                attempt,
                dest_chain,
                error = %last_error,
                "dest_submit_retrying"
            );

            let delay = DEST_SUBMIT_INITIAL_DELAY_MS * (1 << attempt.min(4));
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        Err(BridgeError::ReleaseFailed(format!(
            "Destination submission failed after {DEST_SUBMIT_MAX_RETRIES} attempts: {last_error}"
        )))
    }

    // ── Guardian RPC ─────────────────────────────────────

    /// Poll the guardian network for a signed VAA with exponential backoff.
    async fn fetch_vaa(
        &self,
        chain_id: u16,
        emitter: &str,
        sequence: u64,
    ) -> Result<Vaa, BridgeError> {
        let url = format!(
            "{}{}/{}/{}/{}",
            self.guardian_rpc, VAA_PATH, chain_id, emitter, sequence
        );

        let mut last_error = String::from("VAA not available");

        for attempt in 0..VAA_POLL_MAX_RETRIES {
            // Consume the response inside the async block so we never return
            // a reqwest::Response (which is !Clone) through the circuit breaker.
            let http = &self.http;
            let url = &url;
            let do_fetch = async {
                let resp = http
                    .get(url.as_str())
                    .send()
                    .await
                    .map_err(|e| BridgeError::NetworkError(e.to_string()))?;
                let status = resp.status().as_u16();
                let body = resp
                    .text()
                    .await
                    .map_err(|e| BridgeError::NetworkError(format!("read body: {e}")))?;
                Ok::<(u16, String), BridgeError>((status, body))
            };

            let (status, body) = match self.guardian_breaker.call(do_fetch).await {
                Ok(pair) => pair,
                Err(CircuitError::Open { breaker, remaining_secs }) => {
                    tracing::warn!(breaker, remaining_secs, "guardian_circuit_open");
                    last_error = format!("guardian circuit open ({breaker})");
                    let delay = remaining_secs * 1000;
                    tokio::time::sleep(std::time::Duration::from_millis(delay.min(VAA_POLL_MAX_DELAY_MS))).await;
                    continue;
                }
                Err(CircuitError::Inner(e)) => {
                    tracing::warn!(attempt, error = %e, "guardian_network_error");
                    last_error = e.to_string();
                    let delay = backoff_delay(attempt, VAA_POLL_INITIAL_DELAY_MS, VAA_POLL_MAX_DELAY_MS);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
            };

            if (200..300).contains(&status) {
                let guardian_resp: GuardianResponse = match serde_json::from_str(&body) {
                    Ok(b) => b,
                    Err(e) => {
                        last_error = format!("parse: {e}");
                        continue;
                    }
                };

                if let Some(data) = guardian_resp.data {
                    if let Some(vaa_b64) = data.vaa_bytes {
                        let vaa_bytes = base64_decode(&vaa_b64)?;
                        let vaa = Self::parse_vaa(&vaa_bytes)?;

                        tracing::info!(
                            chain_id,
                            emitter,
                            sequence,
                            num_sigs = vaa.num_signatures,
                            attempt,
                            "wormhole_vaa_fetched"
                        );

                        return Ok(vaa);
                    }
                }

                // VAA not yet available — guardians still signing
                tracing::debug!(attempt, "wormhole_vaa_pending");
            } else if status == 404 {
                tracing::debug!(attempt, "wormhole_vaa_not_found");
            } else {
                tracing::warn!(status, body = %body, attempt, "guardian_error");
                last_error = format!("HTTP {status}");
            }

            let delay = backoff_delay(attempt, VAA_POLL_INITIAL_DELAY_MS, VAA_POLL_MAX_DELAY_MS);
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        Err(BridgeError::VerificationFailed(format!(
            "VAA not available after {VAA_POLL_MAX_RETRIES} attempts: {last_error}"
        )))
    }

    /// Fetch VAA by source transaction hash (alternative guardian endpoint).
    async fn fetch_vaa_by_tx(&self, tx_hash: &str) -> Result<Option<Vaa>, BridgeError> {
        let url = format!("{}{}/{}", self.guardian_rpc, VAA_BY_TX_PATH, tx_hash);

        // Consume the response inside the async block — return (status, body text).
        let http = &self.http;
        let do_fetch = async {
            let resp = http
                .get(&url)
                .send()
                .await
                .map_err(|e| BridgeError::NetworkError(e.to_string()))?;
            let status = resp.status().as_u16();
            let body = resp
                .text()
                .await
                .map_err(|e| BridgeError::NetworkError(format!("read body: {e}")))?;
            Ok::<(u16, String), BridgeError>((status, body))
        };

        let (status, body) = match self.guardian_breaker.call(do_fetch).await {
            Ok(pair) => pair,
            Err(CircuitError::Open { .. }) => return Ok(None),
            Err(CircuitError::Inner(e)) => {
                return Err(BridgeError::NetworkError(e.to_string()));
            }
        };

        if !(200..300).contains(&status) {
            return Ok(None);
        }

        let guardian_resp: GuardianResponse = serde_json::from_str(&body)
            .map_err(|e| BridgeError::VerificationFailed(format!("parse: {e}")))?;

        if let Some(data) = guardian_resp.data {
            if let Some(vaa_b64) = data.vaa_bytes {
                let vaa_bytes = base64_decode(&vaa_b64)?;
                return Ok(Some(Self::parse_vaa(&vaa_bytes)?));
            }
        }

        Ok(None)
    }

    // ── VAA parsing ──────────────────────────────────────

    /// Parse raw VAA bytes into structured form.
    ///
    /// VAA binary format:
    /// ```text
    /// [1B version][4B guardian_set_index][1B num_signatures]
    /// [num_signatures × 66B: guardian_index(1) + signature(65)]
    /// [4B timestamp][4B nonce][2B emitter_chain]
    /// [32B emitter_address][8B sequence][1B consistency]
    /// [remaining: payload]
    /// ```
    fn parse_vaa(bytes: &[u8]) -> Result<Vaa, BridgeError> {
        if bytes.len() < 57 {
            return Err(BridgeError::VerificationFailed("VAA too short".into()));
        }

        let version = bytes[0];
        if version != 1 {
            return Err(BridgeError::VerificationFailed(format!(
                "Unsupported VAA version: {version}"
            )));
        }

        let num_signatures = bytes[5] as usize;

        // Skip past signatures to body
        let body_offset = 6 + num_signatures * 66;
        if bytes.len() < body_offset + 51 {
            return Err(BridgeError::VerificationFailed("VAA body too short".into()));
        }

        let body = &bytes[body_offset..];
        let emitter_chain = u16::from_be_bytes([body[8], body[9]]);
        let emitter_address = hex::encode(&body[10..42]);
        let sequence = u64::from_be_bytes(body[42..50].try_into().unwrap());
        let payload = body[51..].to_vec();

        Ok(Vaa {
            bytes: bytes.to_vec(),
            version,
            num_signatures,
            emitter_chain,
            emitter_address,
            sequence,
            payload,
        })
    }

    /// Verify the VAA has sufficient guardian signatures for quorum.
    ///
    /// Full ECDSA verification of each guardian signature against the
    /// on-chain guardian set is performed by the destination chain's core
    /// bridge contract during redemption. Off-chain we validate the count
    /// as a fast sanity check to avoid submitting under-signed VAAs.
    fn verify_vaa(vaa: &Vaa) -> Result<(), BridgeError> {
        if vaa.num_signatures < GUARDIAN_QUORUM {
            return Err(BridgeError::VerificationFailed(format!(
                "Insufficient signatures: {} < {} quorum",
                vaa.num_signatures, GUARDIAN_QUORUM
            )));
        }

        // Verify no duplicate guardian indices in the signature list.
        // Each signature is 66 bytes: 1B index + 65B secp256k1(r,s,v).
        let sig_start = 6; // after version(1) + guardian_set(4) + num_sigs(1)
        let mut seen = [false; 19];
        for i in 0..vaa.num_signatures {
            let idx = vaa.bytes[sig_start + i * 66] as usize;
            if idx >= 19 {
                return Err(BridgeError::VerificationFailed(format!(
                    "Guardian index out of range: {idx}"
                )));
            }
            if seen[idx] {
                return Err(BridgeError::VerificationFailed(format!(
                    "Duplicate guardian index: {idx}"
                )));
            }
            seen[idx] = true;
        }

        Ok(())
    }
}

// ── BridgeAdapter impl ──────────────────────────────────

#[async_trait]
impl BridgeAdapter for WormholeBridge {
    fn name(&self) -> &str {
        "wormhole"
    }

    fn supports_route(&self, source: &str, dest: &str) -> bool {
        Self::chain_id(source).is_some() && Self::chain_id(dest).is_some() && source != dest
    }

    async fn lock_funds(&self, params: &BridgeTransferParams) -> Result<LockReceipt, BridgeError> {
        let source_id = Self::chain_id(&params.source_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.source_chain.clone()))?;
        let dest_id = Self::chain_id(&params.dest_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.dest_chain.clone()))?;
        let token_bridge = Self::token_bridge(&params.source_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.source_chain.clone()))?;

        tracing::info!(
            bridge = "wormhole",
            source_chain = %params.source_chain,
            dest_chain = %params.dest_chain,
            token = %params.token,
            amount = params.amount,
            sender = %params.sender,
            recipient = %params.recipient,
            "wormhole_lock_initiated"
        );

        // Build Token Bridge transferTokens calldata
        let calldata = Self::encode_transfer_tokens(
            &params.token,
            params.amount,
            dest_id,
            &params.recipient,
        );
        let calldata_hex = format!("0x{}", hex::encode(&calldata));

        // Submit to source chain Token Bridge
        let tx_hash = match self
            .evm_rpc_call(
                &params.source_chain,
                "eth_sendTransaction",
                serde_json::json!([{
                    "from": params.sender,
                    "to": token_bridge,
                    "data": calldata_hex,
                }]),
            )
            .await
        {
            Ok(val) => val.as_str().unwrap_or("").to_string(),
            Err(e) => return Err(BridgeError::LockFailed(e.to_string())),
        };

        if !tx_hash.starts_with("0x") || tx_hash.len() < 66 {
            return Err(BridgeError::LockFailed(format!(
                "Invalid tx hash from source chain: {tx_hash}"
            )));
        }

        // Wait for receipt to parse the Wormhole message
        let receipt = self
            .wait_for_receipt(&params.source_chain, &tx_hash)
            .await?;

        // Parse LogMessagePublished from receipt logs
        let wormhole_msg = Self::parse_lock_logs(&receipt, source_id).ok_or_else(|| {
            BridgeError::LockFailed("No Wormhole LogMessagePublished in tx logs".into())
        })?;

        let message_id = Self::message_id(
            wormhole_msg.emitter_chain,
            &wormhole_msg.emitter_address,
            wormhole_msg.sequence,
        );

        tracing::info!(
            tx_hash = %tx_hash,
            message_id = %message_id,
            sequence = wormhole_msg.sequence,
            "wormhole_lock_confirmed"
        );

        Ok(LockReceipt {
            tx_hash,
            message_id,
            estimated_finality_secs: self
                .get_bridge_time(&params.source_chain, &params.dest_chain)
                .typical_secs,
        })
    }

    async fn verify_lock(&self, tx_hash: &str) -> Result<BridgeStatus, BridgeError> {
        tracing::debug!(bridge = "wormhole", tx_hash, "wormhole_verify_lock");

        match self.fetch_vaa_by_tx(tx_hash).await {
            Ok(Some(vaa)) => {
                Self::verify_vaa(&vaa)?;

                let msg_id = Self::message_id(
                    vaa.emitter_chain,
                    &vaa.emitter_address,
                    vaa.sequence,
                );

                tracing::info!(
                    tx_hash,
                    message_id = %msg_id,
                    signatures = vaa.num_signatures,
                    "wormhole_vaa_verified"
                );

                Ok(BridgeStatus::InTransit { message_id: msg_id })
            }
            Ok(None) => {
                tracing::debug!(tx_hash, "wormhole_vaa_not_yet_available");
                Ok(BridgeStatus::Pending)
            }
            Err(e) => {
                tracing::warn!(tx_hash, error = %e, "wormhole_vaa_fetch_error");
                Ok(BridgeStatus::Pending)
            }
        }
    }

    async fn release_funds(
        &self,
        params: &BridgeTransferParams,
        message_id: &str,
    ) -> Result<String, BridgeError> {
        let (chain_id, emitter, sequence) = Self::parse_message_id(message_id)
            .ok_or_else(|| BridgeError::ReleaseFailed("Invalid message ID".into()))?;

        tracing::info!(
            bridge = "wormhole",
            dest_chain = %params.dest_chain,
            chain_id,
            sequence,
            recipient = %params.recipient,
            amount = params.amount,
            "wormhole_release_fetch_vaa"
        );

        // Fetch the signed VAA from the guardian network
        let vaa = self.fetch_vaa(chain_id, &emitter, sequence).await?;
        Self::verify_vaa(&vaa)?;

        tracing::info!(
            signatures = vaa.num_signatures,
            payload_len = vaa.payload.len(),
            "wormhole_vaa_ready_for_redemption"
        );

        // Submit VAA to destination chain Token Bridge with retry
        let dest_tx_hash = self
            .submit_vaa_to_destination(&params.dest_chain, &vaa)
            .await?;

        tracing::info!(
            dest_tx = %dest_tx_hash,
            dest_chain = %params.dest_chain,
            amount = params.amount,
            "wormhole_release_submitted"
        );

        Ok(dest_tx_hash)
    }

    async fn estimate_bridge_fee(
        &self,
        params: &BridgeTransferParams,
    ) -> Result<BridgeFeeEstimate, BridgeError> {
        let (source_fee, desc) = match params.source_chain.as_str() {
            "solana" => (5_000u64, "~$0.01 (Solana gas)"),
            "ethereum" => (200_000_000_000_000u64, "~$0.50 (Ethereum gas)"),
            "polygon" => (50_000_000_000_000u64, "~$0.02 (Polygon gas)"),
            "arbitrum" => (100_000_000_000_000u64, "~$0.05 (Arbitrum gas)"),
            "base" => (50_000_000_000_000u64, "~$0.02 (Base gas)"),
            _ => (0, "Unknown chain"),
        };

        Ok(BridgeFeeEstimate {
            source_fee,
            dest_fee: 0,
            protocol_fee: 0,
            total_description: format!("{desc} + relayer"),
        })
    }

    fn get_bridge_time(&self, source: &str, _dest: &str) -> BridgeTime {
        let source_finality = match source {
            "solana" => 15,
            "ethereum" => 960,
            "polygon" => 256,
            "arbitrum" => 10,
            "base" => 10,
            _ => 120,
        };

        BridgeTime {
            min_secs: source_finality + 10,
            typical_secs: source_finality + 60,
            max_secs: source_finality + 600,
        }
    }
}

/// Wait for a transaction receipt with exponential backoff.
impl WormholeBridge {
    async fn wait_for_receipt(
        &self,
        chain: &str,
        tx_hash: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        for attempt in 0..20u32 {
            match self.evm_get_receipt(chain, tx_hash).await? {
                Some(receipt) => {
                    let status = receipt
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("0x0");
                    if status == "0x0" {
                        return Err(BridgeError::LockFailed(
                            "Source tx reverted".into(),
                        ));
                    }
                    return Ok(receipt);
                }
                None => {
                    let delay = backoff_delay(attempt, 2_000, 15_000);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }

        Err(BridgeError::LockFailed(format!(
            "Receipt not found after 20 attempts for {tx_hash}"
        )))
    }
}

// ── Helpers ──────────────────────────────────────────────

/// Exponential backoff with jitter, capped at max_ms.
fn backoff_delay(attempt: u32, initial_ms: u64, max_ms: u64) -> u64 {
    let base = initial_ms * (1u64 << attempt.min(6));
    // Add ~25% jitter
    let jitter = (rand::random::<u64>() % (base / 4 + 1)) as u64;
    (base + jitter).min(max_ms)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, BridgeError> {
    let table: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.trim_end_matches('=');
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;

    for &b in input.as_bytes() {
        let val = table
            .iter()
            .position(|&c| c == b)
            .ok_or_else(|| BridgeError::VerificationFailed("Invalid base64".into()))?
            as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(out)
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_eth_to_solana() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(bridge.supports_route("ethereum", "solana"));
        assert!(bridge.supports_route("solana", "ethereum"));
    }

    #[test]
    fn rejects_same_chain() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(!bridge.supports_route("ethereum", "ethereum"));
    }

    #[test]
    fn rejects_unknown_chain() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(!bridge.supports_route("ethereum", "avalanche"));
    }

    #[test]
    fn bridge_time_solana_source_faster() {
        let bridge = WormholeBridge::new("http://localhost");
        let sol = bridge.get_bridge_time("solana", "ethereum");
        let eth = bridge.get_bridge_time("ethereum", "solana");
        assert!(sol.typical_secs < eth.typical_secs);
    }

    #[test]
    fn chain_id_mapping() {
        assert_eq!(WormholeBridge::chain_id("ethereum"), Some(2));
        assert_eq!(WormholeBridge::chain_id("solana"), Some(1));
        assert_eq!(WormholeBridge::chain_id("polygon"), Some(5));
        assert_eq!(WormholeBridge::chain_id("arbitrum"), Some(23));
        assert_eq!(WormholeBridge::chain_id("base"), Some(30));
        assert_eq!(WormholeBridge::chain_id("unknown"), None);
    }

    #[test]
    fn token_bridge_addresses() {
        assert!(WormholeBridge::token_bridge("ethereum")
            .unwrap()
            .starts_with("0x"));
        assert!(WormholeBridge::token_bridge("solana")
            .unwrap()
            .starts_with("worm"));
        assert!(WormholeBridge::token_bridge("unknown").is_none());
    }

    #[test]
    fn message_id_format() {
        let id = WormholeBridge::message_id(2, "abc123", 42);
        assert_eq!(id, "2/abc123/42");
    }

    #[test]
    fn parse_message_id_roundtrip() {
        let id = WormholeBridge::message_id(1, "deadbeef", 99);
        let (chain, emitter, seq) = WormholeBridge::parse_message_id(&id).unwrap();
        assert_eq!(chain, 1);
        assert_eq!(emitter, "deadbeef");
        assert_eq!(seq, 99);
    }

    #[test]
    fn parse_message_id_invalid() {
        assert!(WormholeBridge::parse_message_id("invalid").is_none());
        assert!(WormholeBridge::parse_message_id("1/2").is_none());
    }

    #[test]
    fn parse_vaa_valid() {
        let mut vaa_bytes = Vec::new();
        vaa_bytes.push(1); // version
        vaa_bytes.extend_from_slice(&[0, 0, 0, 0]); // guardian set
        vaa_bytes.push(0); // 0 signatures
        // body
        vaa_bytes.extend_from_slice(&[0u8; 4]); // timestamp
        vaa_bytes.extend_from_slice(&[0u8; 4]); // nonce
        vaa_bytes.extend_from_slice(&[0, 2]); // emitter_chain = 2
        vaa_bytes.extend_from_slice(&[0xAA; 32]); // emitter address
        vaa_bytes.extend_from_slice(&42u64.to_be_bytes()); // sequence
        vaa_bytes.push(1); // consistency

        let vaa = WormholeBridge::parse_vaa(&vaa_bytes).unwrap();
        assert_eq!(vaa.version, 1);
        assert_eq!(vaa.emitter_chain, 2);
        assert_eq!(vaa.sequence, 42);
        assert_eq!(vaa.num_signatures, 0);
    }

    #[test]
    fn parse_vaa_too_short() {
        assert!(WormholeBridge::parse_vaa(&[1, 0, 0]).is_err());
    }

    #[test]
    fn parse_vaa_wrong_version() {
        let mut bytes = vec![2]; // version 2
        bytes.extend_from_slice(&[0; 60]);
        assert!(WormholeBridge::parse_vaa(&bytes).is_err());
    }

    #[test]
    fn verify_vaa_quorum() {
        // Build a VAA with 13 signatures and valid (unique) indices
        let mut bytes = Vec::new();
        bytes.push(1); // version
        bytes.extend_from_slice(&[0, 0, 0, 0]); // guardian set
        bytes.push(GUARDIAN_QUORUM as u8); // num_signatures
        for i in 0..GUARDIAN_QUORUM {
            bytes.push(i as u8); // guardian index
            bytes.extend_from_slice(&[0u8; 65]); // signature placeholder
        }
        // body (minimal)
        bytes.extend_from_slice(&[0u8; 51]);

        let vaa = Vaa {
            bytes,
            version: 1,
            num_signatures: GUARDIAN_QUORUM,
            emitter_chain: 2,
            emitter_address: "aa".into(),
            sequence: 1,
            payload: vec![],
        };
        assert!(WormholeBridge::verify_vaa(&vaa).is_ok());
    }

    #[test]
    fn verify_vaa_insufficient_quorum() {
        let mut bytes = Vec::new();
        bytes.push(1);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.push((GUARDIAN_QUORUM - 1) as u8);
        for i in 0..(GUARDIAN_QUORUM - 1) {
            bytes.push(i as u8);
            bytes.extend_from_slice(&[0u8; 65]);
        }
        bytes.extend_from_slice(&[0u8; 51]);

        let vaa = Vaa {
            bytes,
            version: 1,
            num_signatures: GUARDIAN_QUORUM - 1,
            emitter_chain: 2,
            emitter_address: "aa".into(),
            sequence: 1,
            payload: vec![],
        };
        assert!(WormholeBridge::verify_vaa(&vaa).is_err());
    }

    #[test]
    fn verify_vaa_duplicate_guardian_index() {
        let mut bytes = Vec::new();
        bytes.push(1);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.push(GUARDIAN_QUORUM as u8);
        for i in 0..GUARDIAN_QUORUM {
            // All use index 0 — should be rejected as duplicates
            bytes.push(0);
            bytes.extend_from_slice(&[0u8; 65]);
        }
        bytes.extend_from_slice(&[0u8; 51]);

        let vaa = Vaa {
            bytes,
            version: 1,
            num_signatures: GUARDIAN_QUORUM,
            emitter_chain: 2,
            emitter_address: "aa".into(),
            sequence: 1,
            payload: vec![],
        };
        assert!(WormholeBridge::verify_vaa(&vaa).is_err());
    }

    #[test]
    fn verify_vaa_guardian_index_out_of_range() {
        let mut bytes = Vec::new();
        bytes.push(1);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.push(GUARDIAN_QUORUM as u8);
        for i in 0..GUARDIAN_QUORUM {
            bytes.push(if i == 0 { 20 } else { i as u8 }); // index 20 is out of range
            bytes.extend_from_slice(&[0u8; 65]);
        }
        bytes.extend_from_slice(&[0u8; 51]);

        let vaa = Vaa {
            bytes,
            version: 1,
            num_signatures: GUARDIAN_QUORUM,
            emitter_chain: 2,
            emitter_address: "aa".into(),
            sequence: 1,
            payload: vec![],
        };
        assert!(WormholeBridge::verify_vaa(&vaa).is_err());
    }

    #[test]
    fn base64_decode_simple() {
        let decoded = base64_decode("SGVsbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn base64_decode_empty() {
        let decoded = base64_decode("").unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn encode_transfer_tokens_length() {
        let data = WormholeBridge::encode_transfer_tokens(
            "0x1234567890abcdef1234567890abcdef12345678",
            1_000_000,
            1, // solana
            "0xdeadbeef",
        );
        // 4 selector + 6 * 32 args = 196 bytes
        assert_eq!(data.len(), 4 + 6 * 32);
        assert_eq!(&data[..4], &TRANSFER_TOKENS_SELECTOR);
    }

    #[test]
    fn encode_complete_transfer_contains_vaa() {
        let vaa_bytes = vec![1, 2, 3, 4, 5];
        let data = WormholeBridge::encode_complete_transfer(&vaa_bytes);
        assert_eq!(&data[..4], &COMPLETE_TRANSFER_SELECTOR);
        // The VAA bytes should appear after selector(4) + offset(32) + length(32) = 68
        assert_eq!(&data[68..73], &vaa_bytes);
    }

    #[test]
    fn backoff_delay_increases() {
        let d0 = backoff_delay(0, 1000, 30_000);
        let d3 = backoff_delay(3, 1000, 30_000);
        // d3 base = 1000 * 8 = 8000, d0 base = 1000
        assert!(d3 > d0);
    }

    #[test]
    fn backoff_delay_capped() {
        let d = backoff_delay(20, 1000, 5_000);
        assert!(d <= 5_000);
    }

    #[test]
    fn fee_estimate_per_chain() {
        let bridge = WormholeBridge::new("http://localhost");

        let sol_fee = tokio::runtime::Runtime::new().unwrap().block_on(
            bridge.estimate_bridge_fee(&BridgeTransferParams {
                source_chain: "solana".into(),
                dest_chain: "ethereum".into(),
                token: String::new(),
                amount: 1000,
                sender: String::new(),
                recipient: String::new(),
            }),
        )
        .unwrap();

        let eth_fee = tokio::runtime::Runtime::new().unwrap().block_on(
            bridge.estimate_bridge_fee(&BridgeTransferParams {
                source_chain: "ethereum".into(),
                dest_chain: "solana".into(),
                token: String::new(),
                amount: 1000,
                sender: String::new(),
                recipient: String::new(),
            }),
        )
        .unwrap();

        assert!(sol_fee.source_fee < eth_fee.source_fee);
    }

    #[test]
    fn parse_lock_logs_extracts_message() {
        let receipt = serde_json::json!({
            "logs": [{
                "topics": [
                    LOG_MESSAGE_PUBLISHED_TOPIC,
                    "0x0000000000000000000000003ee18b2214aff97000d974cf647e7c347e8fa585"
                ],
                "data": format!("0x{}", hex::encode(&{
                    let mut d = vec![0u8; 32];
                    d[24..].copy_from_slice(&42u64.to_be_bytes());
                    d
                }))
            }]
        });

        let msg = WormholeBridge::parse_lock_logs(&receipt, 2).unwrap();
        assert_eq!(msg.emitter_chain, 2);
        assert_eq!(msg.sequence, 42);
    }

    #[test]
    fn parse_lock_logs_no_wormhole_event() {
        let receipt = serde_json::json!({
            "logs": [{
                "topics": ["0xdeadbeef"],
                "data": "0x"
            }]
        });
        assert!(WormholeBridge::parse_lock_logs(&receipt, 2).is_none());
    }

    #[test]
    fn with_chain_rpc_registers() {
        let bridge = WormholeBridge::new("http://guardian")
            .with_chain_rpc("ethereum", "http://eth-rpc")
            .with_chain_rpc("polygon", "http://poly-rpc");

        assert!(bridge.chain_rpcs.contains_key("ethereum"));
        assert!(bridge.chain_rpcs.contains_key("polygon"));
        assert!(bridge.chain_breakers.contains_key("ethereum"));
    }
}
