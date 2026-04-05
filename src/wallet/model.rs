use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Wallet ─────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Wallet {
    pub id: Uuid,
    pub account_id: Uuid,
    pub address: String,
    pub chain: String,
    pub encrypted_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// Public view — never exposes encrypted_key or nonce.
#[derive(Debug, Clone, Serialize)]
pub struct WalletPublic {
    pub id: Uuid,
    pub account_id: Uuid,
    pub address: String,
    pub chain: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Wallet> for WalletPublic {
    fn from(w: Wallet) -> Self {
        Self {
            id: w.id,
            account_id: w.account_id,
            address: w.address,
            chain: w.chain,
            active: w.active,
            created_at: w.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWalletRequest {
    pub account_id: Uuid,
    #[serde(default = "default_chain")]
    pub chain: String,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

// ── Transaction ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "tx_status", rename_all = "lowercase")]
pub enum TxStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
    Dropped,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TransactionRecord {
    pub id: Uuid,
    pub fill_id: Option<Uuid>,
    pub from_address: String,
    pub to_address: String,
    pub chain: String,
    pub tx_hash: Option<String>,
    pub amount: i64,
    pub asset: String,
    pub status: TxStatus,
    pub gas_price: Option<i64>,
    pub gas_used: Option<i64>,
    pub block_number: Option<i64>,
    pub confirmations: i32,
    pub error: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Payload to build a transaction before signing.
#[derive(Debug, Clone, Serialize)]
pub struct TxPayload {
    pub from: String,
    pub to: String,
    pub value: i64,
    pub asset: String,
    pub chain: String,
    pub data: Vec<u8>,
}
