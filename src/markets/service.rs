use chrono::Utc;
use uuid::Uuid;

use super::model::{CreateMarketRequest, Market};
use super::repository::MarketRepository;

#[derive(Debug)]
pub enum MarketError {
    NotFound,
    DbError(sqlx::Error),
}

impl std::fmt::Display for MarketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketError::NotFound => write!(f, "Market not found"),
            MarketError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for MarketError {
    fn from(e: sqlx::Error) -> Self {
        MarketError::DbError(e)
    }
}

pub struct MarketService {
    repo: MarketRepository,
}

impl MarketService {
    pub fn new(repo: MarketRepository) -> Self {
        Self { repo }
    }

    pub async fn create_market(&self, req: CreateMarketRequest) -> Result<Market, MarketError> {
        let market = Market {
            id: Uuid::new_v4(),
            base_asset: req.base_asset,
            quote_asset: req.quote_asset,
            tick_size: req.tick_size,
            min_order_size: req.min_order_size,
            fee_rate: req.fee_rate,
            created_at: Utc::now(),
        };
        self.repo.insert(&market).await?;
        Ok(market)
    }

    pub async fn get_market(&self, market_id: Uuid) -> Result<Market, MarketError> {
        self.repo
            .find_by_id(market_id)
            .await?
            .ok_or(MarketError::NotFound)
    }

    pub async fn list_markets(&self) -> Result<Vec<Market>, MarketError> {
        Ok(self.repo.find_all().await?)
    }
}
