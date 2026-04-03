use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Candle, MarketTrade, PriceLevel};

pub struct MarketDataRepository {
    pool: PgPool,
}

impl MarketDataRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert_trade(&self, trade: &MarketTrade) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO market_trades (id, market_id, buyer_account_id, seller_account_id, price, qty, fee, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(trade.id)
        .bind(trade.market_id)
        .bind(trade.buyer_account_id)
        .bind(trade.seller_account_id)
        .bind(trade.price)
        .bind(trade.qty)
        .bind(trade.fee)
        .bind(trade.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_trades(
        &self,
        market_id: Uuid,
        limit: i64,
    ) -> Result<Vec<MarketTrade>, sqlx::Error> {
        sqlx::query_as::<_, MarketTrade>(
            "SELECT * FROM market_trades WHERE market_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(market_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_bids(&self, market_id: Uuid) -> Result<Vec<PriceLevel>, sqlx::Error> {
        sqlx::query_as::<_, PriceLevel>(
            "SELECT price, SUM(qty)::BIGINT as qty FROM market_trades
             WHERE market_id = $1
             GROUP BY price ORDER BY price DESC LIMIT 50",
        )
        .bind(market_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_asks(&self, market_id: Uuid) -> Result<Vec<PriceLevel>, sqlx::Error> {
        sqlx::query_as::<_, PriceLevel>(
            "SELECT price, SUM(qty)::BIGINT as qty FROM market_trades
             WHERE market_id = $1
             GROUP BY price ORDER BY price ASC LIMIT 50",
        )
        .bind(market_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_candles(
        &self,
        market_id: Uuid,
        interval: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Candle>, sqlx::Error> {
        let bucket_interval = match interval {
            "1m" => "1 minute",
            "5m" => "5 minutes",
            "15m" => "15 minutes",
            "1h" => "1 hour",
            "4h" => "4 hours",
            "1d" => "1 day",
            _ => "1 minute",
        };

        let query = format!(
            "SELECT
                $1::UUID as market_id,
                (array_agg(price ORDER BY created_at ASC))[1] as open,
                MAX(price) as high,
                MIN(price) as low,
                (array_agg(price ORDER BY created_at DESC))[1] as close,
                SUM(qty)::BIGINT as volume,
                COUNT(*)::BIGINT as trade_count,
                date_trunc('minute', created_at) -
                    (EXTRACT(minute FROM created_at)::INT % {m}) * INTERVAL '1 minute' as bucket
            FROM market_trades
            WHERE market_id = $1 AND created_at >= $2
            GROUP BY bucket
            ORDER BY bucket ASC",
            m = match interval {
                "1m" => 1,
                "5m" => 5,
                "15m" => 15,
                _ => 1,
            }
        );

        // For hour/day intervals, use a simpler bucketing
        let candles = match interval {
            "1h" | "4h" | "1d" => {
                sqlx::query_as::<_, Candle>(&format!(
                    "SELECT
                        $1::UUID as market_id,
                        (array_agg(price ORDER BY created_at ASC))[1] as open,
                        MAX(price) as high,
                        MIN(price) as low,
                        (array_agg(price ORDER BY created_at DESC))[1] as close,
                        SUM(qty)::BIGINT as volume,
                        COUNT(*)::BIGINT as trade_count,
                        date_trunc('{unit}', created_at) as bucket
                    FROM market_trades
                    WHERE market_id = $1 AND created_at >= $2
                    GROUP BY bucket
                    ORDER BY bucket ASC",
                    unit = match interval {
                        "1h" | "4h" => "hour",
                        "1d" => "day",
                        _ => "hour",
                    }
                ))
                .bind(market_id)
                .bind(since)
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as::<_, Candle>(&query)
                    .bind(market_id)
                    .bind(since)
                    .fetch_all(&self.pool)
                    .await?
            }
        };

        Ok(candles)
    }
}
