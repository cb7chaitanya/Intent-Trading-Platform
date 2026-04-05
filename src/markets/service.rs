use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::cache::service::{CacheService, CacheTtl};

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
    cache: Option<Arc<CacheService>>,
}

impl MarketService {
    pub fn new(repo: MarketRepository) -> Self {
        Self { repo, cache: None }
    }

    pub fn with_cache(mut self, cache: Arc<CacheService>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub async fn create_market(&self, req: CreateMarketRequest) -> Result<Market, MarketError> {
        let market = Market {
            id: Uuid::new_v4(),
            base_asset: req.base_asset,
            quote_asset: req.quote_asset,
            tick_size: req.tick_size,
            min_order_size: req.min_order_size,
            fee_rate: req.fee_rate,
            chain: req.chain.unwrap_or_else(|| "ethereum".to_string()),
            created_at: Utc::now(),
        };
        self.repo.insert(&market).await?;

        // Invalidate market list cache
        if let Some(cache) = &self.cache {
            cache.invalidate("markets", "all").await;
        }

        Ok(market)
    }

    pub async fn get_market(&self, market_id: Uuid) -> Result<Market, MarketError> {
        let key = market_id.to_string();

        if let Some(cache) = &self.cache {
            if let Some(market) = cache.get::<Market>("market", &key).await {
                return Ok(market);
            }
        }

        let market = self.repo.find_by_id(market_id).await?.ok_or(MarketError::NotFound)?;

        if let Some(cache) = &self.cache {
            cache.set("market", &key, &market, CacheTtl::MARKETS).await;
        }

        Ok(market)
    }

    pub async fn list_markets(&self) -> Result<Vec<Market>, MarketError> {
        if let Some(cache) = &self.cache {
            if let Some(markets) = cache.get::<Vec<Market>>("markets", "all").await {
                return Ok(markets);
            }
        }

        let markets = self.repo.find_all().await?;

        if let Some(cache) = &self.cache {
            cache.set("markets", "all", &markets, CacheTtl::MARKETS).await;
        }

        Ok(markets)
    }
}
