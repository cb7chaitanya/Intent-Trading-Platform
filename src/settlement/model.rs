use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::balances::model::Asset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "trade_status", rename_all = "lowercase")]
pub enum TradeStatus {
    Pending,
    Settled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Trade {
    pub id: Uuid,
    pub buyer_account_id: Uuid,
    pub seller_account_id: Uuid,
    pub solver_account_id: Uuid,
    pub asset_in: Asset,
    pub asset_out: Asset,
    pub amount_in: i64,
    pub amount_out: i64,
    pub platform_fee: i64,
    pub solver_fee: i64,
    pub status: TradeStatus,
    pub created_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTradeRequest {
    pub buyer_account_id: Uuid,
    pub seller_account_id: Uuid,
    pub solver_account_id: Uuid,
    pub asset_in: Asset,
    pub asset_out: Asset,
    pub amount_in: i64,
    pub amount_out: i64,
    pub platform_fee: i64,
    pub solver_fee: i64,
}
