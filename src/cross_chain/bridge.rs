//! Bridge adapter trait for cross-chain token transfers.
//!
//! Each bridge protocol (Wormhole, LayerZero, etc.) implements this trait.
//! The cross-chain worker selects the appropriate bridge based on the
//! source/destination chain pair.

use async_trait::async_trait;
use serde::Serialize;

// ── Types ────────────────────────────────────────────────

/// Result of locking funds on the source chain.
#[derive(Debug, Clone, Serialize)]
pub struct LockReceipt {
    /// Transaction hash on the source chain.
    pub tx_hash: String,
    /// Bridge-specific message/sequence ID for tracking.
    pub message_id: String,
    /// Estimated time to finality in seconds.
    pub estimated_finality_secs: u64,
}

/// Status of a bridge lock/transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum BridgeStatus {
    /// Lock transaction submitted, waiting for source chain confirmations.
    Pending,
    /// Source chain confirmed, bridge message in transit.
    InTransit { message_id: String },
    /// Funds released on destination chain.
    Completed { dest_tx_hash: String },
    /// Bridge transfer failed.
    Failed { reason: String },
}

/// Fee estimate for a bridge transfer.
#[derive(Debug, Clone, Serialize)]
pub struct BridgeFeeEstimate {
    /// Fee in the source chain's native token (lamports, wei, etc.).
    pub source_fee: u64,
    /// Fee in the destination chain's native token.
    pub dest_fee: u64,
    /// Relayer/protocol fee in the transferred token.
    pub protocol_fee: u64,
    /// Human-readable total fee description.
    pub total_description: String,
}

/// Estimated bridge transfer time.
#[derive(Debug, Clone, Serialize)]
pub struct BridgeTime {
    /// Minimum expected time in seconds.
    pub min_secs: u64,
    /// Typical expected time in seconds.
    pub typical_secs: u64,
    /// Maximum expected time in seconds (before timeout).
    pub max_secs: u64,
}

/// Parameters for a bridge lock or release operation.
#[derive(Debug, Clone)]
pub struct BridgeTransferParams {
    pub source_chain: String,
    pub dest_chain: String,
    pub token: String,
    pub amount: u64,
    pub sender: String,
    pub recipient: String,
}

/// Errors from bridge operations.
#[derive(Debug)]
pub enum BridgeError {
    /// The bridge doesn't support this chain pair.
    UnsupportedRoute(String),
    /// Source chain transaction failed.
    LockFailed(String),
    /// Verification of a lock couldn't find or confirm the tx.
    VerificationFailed(String),
    /// Destination release failed.
    ReleaseFailed(String),
    /// Network or RPC error.
    NetworkError(String),
    /// Unexpected error.
    Other(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::UnsupportedRoute(r) => write!(f, "Unsupported route: {r}"),
            BridgeError::LockFailed(e) => write!(f, "Lock failed: {e}"),
            BridgeError::VerificationFailed(e) => write!(f, "Verification failed: {e}"),
            BridgeError::ReleaseFailed(e) => write!(f, "Release failed: {e}"),
            BridgeError::NetworkError(e) => write!(f, "Network error: {e}"),
            BridgeError::Other(e) => write!(f, "Bridge error: {e}"),
        }
    }
}

// ── Trait ─────────────────────────────────────────────────

/// Abstraction over cross-chain bridge protocols.
///
/// The cross-chain settlement flow is:
/// 1. `lock_funds` on the source chain → get LockReceipt
/// 2. `verify_lock` to confirm the lock is finalized
/// 3. `release_funds` on the destination chain
///
/// Each bridge implementation handles its own message passing
/// (VAAs for Wormhole, packets for LayerZero, etc.).
#[async_trait]
pub trait BridgeAdapter: Send + Sync {
    /// Human-readable bridge name ("wormhole", "layerzero").
    fn name(&self) -> &str;

    /// Whether this bridge supports a given source→dest chain pair.
    fn supports_route(&self, source_chain: &str, dest_chain: &str) -> bool;

    /// Lock (escrow) funds on the source chain.
    /// Returns a receipt with tx_hash and bridge message ID.
    async fn lock_funds(&self, params: &BridgeTransferParams) -> Result<LockReceipt, BridgeError>;

    /// Verify that a lock transaction has been confirmed and the bridge
    /// message is in transit or delivered.
    async fn verify_lock(&self, tx_hash: &str) -> Result<BridgeStatus, BridgeError>;

    /// Release funds on the destination chain after the bridge message
    /// has been delivered. Some bridges do this automatically (relayer),
    /// others require an explicit claim transaction.
    async fn release_funds(&self, params: &BridgeTransferParams, message_id: &str) -> Result<String, BridgeError>;

    /// Estimate fees for a bridge transfer.
    async fn estimate_bridge_fee(&self, params: &BridgeTransferParams) -> Result<BridgeFeeEstimate, BridgeError>;

    /// Estimate transfer time for a bridge route.
    fn get_bridge_time(&self, source_chain: &str, dest_chain: &str) -> BridgeTime;
}
