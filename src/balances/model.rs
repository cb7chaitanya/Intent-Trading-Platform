use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "asset_type", rename_all = "UPPERCASE")]
pub enum Asset {
    USDC,
    ETH,
    BTC,
    SOL,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Balance {
    pub id: Uuid,
    pub account_id: Uuid,
    pub asset: Asset,
    pub available_balance: i64,
    pub locked_balance: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct DepositRequest {
    pub account_id: Uuid,
    pub asset: Asset,
    pub amount: i64,
}

#[derive(Debug, Deserialize)]
pub struct WithdrawRequest {
    pub account_id: Uuid,
    pub asset: Asset,
    pub amount: i64,
}

#[derive(Debug, Deserialize)]
pub struct LockRequest {
    pub account_id: Uuid,
    pub asset: Asset,
    pub amount: i64,
}

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub from_account_id: Uuid,
    pub to_account_id: Uuid,
    pub asset: Asset,
    pub amount: i64,
}
