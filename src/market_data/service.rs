use chrono::{Duration, Utc};
use uuid::Uuid;

use super::model::{Candle, MarketTrade, OrderBookSnapshot};
use super::repository::MarketDataRepository;

#[derive(Debug)]
pub enum MarketDataError {
    InvalidInterval(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for MarketDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketDataError::InvalidInterval(i) => write!(f, "Invalid interval: {i}"),
            MarketDataError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for MarketDataError {
    fn from(e: sqlx::Error) -> Self {
        MarketDataError::DbError(e)
    }
}

pub struct MarketDataService {
    repo: MarketDataRepository,
}

impl MarketDataService {
    pub fn new(repo: MarketDataRepository) -> Self {
        Self { repo }
    }

    pub async fn record_trade(&self, trade: MarketTrade) -> Result<(), MarketDataError> {
        self.repo.insert_trade(&trade).await?;
        Ok(())
    }

    pub async fn get_trades(
        &self,
        market_id: Uuid,
        limit: i64,
    ) -> Result<Vec<MarketTrade>, MarketDataError> {
        let limit = limit.clamp(1, 1000);
        Ok(self.repo.get_trades(market_id, limit).await?)
    }

    pub async fn get_orderbook_snapshot(
        &self,
        market_id: Uuid,
    ) -> Result<OrderBookSnapshot, MarketDataError> {
        let bids = self.repo.get_bids(market_id).await?;
        let asks = self.repo.get_asks(market_id).await?;
        Ok(OrderBookSnapshot {
            market_id,
            bids,
            asks,
            timestamp: Utc::now(),
        })
    }

    pub async fn generate_candles(
        &self,
        market_id: Uuid,
        interval: &str,
    ) -> Result<Vec<Candle>, MarketDataError> {
        let lookback = match interval {
            "1m" => Duration::hours(6),
            "5m" => Duration::hours(24),
            "15m" => Duration::days(3),
            "1h" => Duration::days(7),
            "4h" => Duration::days(30),
            "1d" => Duration::days(90),
            other => return Err(MarketDataError::InvalidInterval(other.to_string())),
        };
        let since = Utc::now() - lookback;
        Ok(self.repo.get_candles(market_id, interval, since).await?)
    }
}
