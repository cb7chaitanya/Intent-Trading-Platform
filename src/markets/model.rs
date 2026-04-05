use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::balances::model::Asset;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Market {
    pub id: Uuid,
    pub base_asset: Asset,
    pub quote_asset: Asset,
    pub tick_size: i64,
    pub min_order_size: i64,
    pub fee_rate: f64,
    pub chain: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMarketRequest {
    pub base_asset: Asset,
    pub quote_asset: Asset,
    pub tick_size: i64,
    pub min_order_size: i64,
    pub fee_rate: f64,
    pub chain: Option<String>,
}
