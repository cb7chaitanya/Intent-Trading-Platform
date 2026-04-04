use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Serialize;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::markets::service::MarketService;
use std::sync::Arc;

const UPDATE_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MarketPrice {
    pub market_id: Uuid,
    pub price: i64,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

pub struct OracleService {
    pool: PgPool,
    market_service: Arc<MarketService>,
}

impl OracleService {
    pub fn new(pool: PgPool, market_service: Arc<MarketService>) -> Self {
        Self {
            pool,
            market_service,
        }
    }

    /// Get the latest price for a market.
    pub async fn get_price(&self, market_id: &Uuid) -> Option<MarketPrice> {
        sqlx::query_as::<_, MarketPrice>(
            "SELECT * FROM market_prices WHERE market_id = $1",
        )
        .bind(market_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
    }

    /// Get all latest prices.
    pub async fn get_all_prices(&self) -> Vec<MarketPrice> {
        sqlx::query_as::<_, MarketPrice>(
            "SELECT * FROM market_prices ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    /// Get the latest price as i64 for a market, or None if not available.
    pub async fn get_price_value(&self, market_id: &Uuid) -> Option<i64> {
        self.get_price(market_id).await.map(|p| p.price)
    }

    /// Fetch price from external source (mock implementation).
    /// In production, replace with actual API calls to price feeds.
    async fn fetch_external_price(&self, market_id: &Uuid) -> Option<i64> {
        // Mock: generate a price based on the market
        // In production: call Binance/Coingecko/Chainlink API
        let base_prices: std::collections::HashMap<&str, i64> = [
            ("ETH", 3500_00),  // $3500.00 in cents
            ("BTC", 68000_00), // $68000.00
            ("SOL", 180_00),   // $180.00
        ]
        .into();

        let market = self.market_service.get_market(*market_id).await.ok()?;
        let base_asset = format!("{:?}", market.base_asset);

        let base = base_prices.get(base_asset.as_str()).copied()?;

        // Add random jitter ±2%
        let jitter = {
            let mut rng = rand::rng();
            rng.random_range(0.98..1.02)
        };

        Some((base as f64 * jitter) as i64)
    }

    /// Update prices for all markets.
    async fn update_all_prices(&self) {
        let markets = match self.market_service.list_markets().await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "Failed to list markets for oracle update");
                return;
            }
        };

        let now = Utc::now();

        for market in &markets {
            let price = match self.fetch_external_price(&market.id).await {
                Some(p) => p,
                None => continue,
            };

            let result = sqlx::query(
                "INSERT INTO market_prices (market_id, price, source, updated_at)
                 VALUES ($1, $2, 'mock', $3)
                 ON CONFLICT (market_id) DO UPDATE SET
                    price = EXCLUDED.price,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(market.id)
            .bind(price)
            .bind(now)
            .execute(&self.pool)
            .await;

            match result {
                Ok(_) => {
                    tracing::debug!(
                        market_id = %market.id,
                        price = price,
                        "oracle_price_updated"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        market_id = %market.id,
                        error = %e,
                        "Failed to update oracle price"
                    );
                }
            }
        }

        tracing::info!(markets = markets.len(), "oracle_prices_updated");
    }

    /// Background worker that updates prices periodically.
    pub async fn run_price_feed(self: Arc<Self>, cancel: CancellationToken) {
        tracing::info!("Oracle price feed started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(UPDATE_INTERVAL_SECS)) => {}
                _ = cancel.cancelled() => {
                    tracing::info!("Oracle price feed shutting down");
                    return;
                }
            }

            self.update_all_prices().await;
        }
    }
}
