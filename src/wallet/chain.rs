use async_trait::async_trait;
use serde::Serialize;

// ── Types ────────────────────────────────────────────────

/// Status of an on-chain transaction.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TxState {
    Pending,
    Confirmed {
        block: u64,
        confirmations: u32,
    },
    Failed {
        reason: String,
    },
}

/// Fee estimate returned before sending.
#[derive(Debug, Clone, Serialize)]
pub struct FeeEstimate {
    /// Base fee in the chain's native unit (wei, lamports, etc.)
    pub base_fee: u64,
    /// Priority/tip fee.
    pub priority_fee: u64,
    /// Estimated total cost in native units.
    pub total: u64,
    /// Native unit name for display ("wei", "lamports").
    pub unit: String,
}

/// Data needed to build a settlement transaction.
#[derive(Debug, Clone)]
pub struct SettlementData {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub token: String,
    pub chain: String,
}

/// An unsigned transaction blob, chain-specific encoding.
#[derive(Debug, Clone)]
pub struct UnsignedTx {
    pub chain: String,
    pub data: Vec<u8>,
}

/// A signed transaction ready for broadcast.
#[derive(Debug, Clone)]
pub struct SignedTx {
    pub chain: String,
    pub data: Vec<u8>,
}

/// Errors from chain adapter operations.
#[derive(Debug)]
pub enum ChainError {
    Rpc(String),
    Signing(String),
    Unsupported(String),
    Other(String),
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::Rpc(e) => write!(f, "RPC: {e}"),
            ChainError::Signing(e) => write!(f, "Signing: {e}"),
            ChainError::Unsupported(e) => write!(f, "Unsupported: {e}"),
            ChainError::Other(e) => write!(f, "{e}"),
        }
    }
}

// ── Trait ─────────────────────────────────────────────────

/// Abstraction over blockchain-specific operations.
///
/// Each supported chain (Ethereum, Solana, ...) provides an implementation.
/// The `WalletService` and `SettlementEngine` interact exclusively through
/// this trait, so adding a new chain requires only a new impl + registration.
#[async_trait]
pub trait ChainAdapter: Send + Sync {
    /// Human-readable chain name ("ethereum", "solana").
    fn chain_name(&self) -> &str;

    /// Required block confirmations before finalizing.
    fn required_confirmations(&self) -> u32;

    /// Seconds after submission with no receipt before marking as dropped.
    /// Ethereum: ~600s (10 min), Solana: ~120s (blockhash expiry).
    fn drop_timeout_secs(&self) -> i64 {
        600
    }

    /// Send a signed transaction and return the tx hash.
    async fn send_transaction(&self, tx: &SignedTx) -> Result<String, ChainError>;

    /// Check current status of a previously submitted transaction.
    async fn get_transaction_status(&self, tx_hash: &str) -> Result<TxState, ChainError>;

    /// Estimate fees for a transaction before sending.
    async fn estimate_fees(&self, data: &SettlementData) -> Result<FeeEstimate, ChainError>;

    /// Query on-chain token balance for an address.
    async fn get_balance(&self, address: &str, token: &str) -> Result<u64, ChainError>;

    /// Build an unsigned settlement transaction from business data.
    async fn build_settlement_tx(
        &self,
        data: &SettlementData,
    ) -> Result<UnsignedTx, ChainError>;

    /// Sign an unsigned transaction with the given private key bytes.
    fn sign_transaction(
        &self,
        unsigned_tx: &UnsignedTx,
        private_key: &[u8; 32],
    ) -> Result<SignedTx, ChainError>;
}
