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
    /// Chain this market settles on ("ethereum", "solana").
    pub chain: String,
    /// On-chain contract/program address for settlement.
    pub settlement_contract: Option<String>,
    /// Token mint/contract address for the base asset on this chain.
    pub base_token_mint: Option<String>,
    /// Token mint/contract address for the quote asset on this chain.
    pub quote_token_mint: Option<String>,
    /// Decimal places for the base asset on this chain.
    pub base_decimals: i16,
    /// Decimal places for the quote asset on this chain.
    pub quote_decimals: i16,
    pub created_at: DateTime<Utc>,
}

impl Market {
    /// Convert a human-readable amount to on-chain base units using base_decimals.
    pub fn to_base_units(&self, amount: f64) -> u64 {
        (amount * 10f64.powi(self.base_decimals as i32)) as u64
    }

    /// Convert a human-readable amount to on-chain quote units using quote_decimals.
    pub fn to_quote_units(&self, amount: f64) -> u64 {
        (amount * 10f64.powi(self.quote_decimals as i32)) as u64
    }

    pub fn is_solana(&self) -> bool {
        self.chain == "solana"
    }

    pub fn is_ethereum(&self) -> bool {
        self.chain == "ethereum"
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMarketRequest {
    pub base_asset: Asset,
    pub quote_asset: Asset,
    pub tick_size: i64,
    pub min_order_size: i64,
    pub fee_rate: f64,
    pub chain: Option<String>,
    pub settlement_contract: Option<String>,
    pub base_token_mint: Option<String>,
    pub quote_token_mint: Option<String>,
    pub base_decimals: Option<i16>,
    pub quote_decimals: Option<i16>,
}
