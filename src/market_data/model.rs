use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MarketTrade {
    pub id: Uuid,
    pub market_id: Uuid,
    pub buyer_account_id: Uuid,
    pub seller_account_id: Uuid,
    pub price: i64,
    pub qty: i64,
    pub fee: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PriceLevel {
    pub price: i64,
    pub qty: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookSnapshot {
    pub market_id: Uuid,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Candle {
    pub market_id: Uuid,
    pub open: i64,
    pub high: i64,
    pub low: i64,
    pub close: i64,
    pub volume: i64,
    pub trade_count: i64,
    pub bucket: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct TradesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct CandlesQuery {
    #[serde(default = "default_interval")]
    pub interval: String,
}

fn default_interval() -> String {
    "1m".to_string()
}
