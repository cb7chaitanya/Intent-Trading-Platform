use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::markets::model::Market;
use crate::markets::service::MarketService;
use std::sync::Arc;

/// How often the oracle worker fetches prices.
const UPDATE_INTERVAL_SECS: u64 = 2;

/// Maximum allowed deviation between sources (40%) before flagging anomaly.
const ANOMALY_THRESHOLD: f64 = 0.40;

/// How many historical prices to keep per market for TWAP queries.
const HISTORY_RETENTION_ROWS: i64 = 1000;

// ── Public types ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MarketPrice {
    pub market_id: Uuid,
    pub price: i64,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TwapPrice {
    pub market_id: Uuid,
    pub twap: f64,
    pub samples: i64,
    pub window_secs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceAnomaly {
    pub market_id: Uuid,
    pub prices: Vec<SourcePrice>,
    pub max_deviation: f64,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourcePrice {
    pub source: String,
    pub price: f64,
}

// ── Exchange response types ──────────────────────────────

#[derive(Debug, Deserialize)]
struct BinanceTicker {
    #[serde(rename = "lastPrice")]
    last_price: Option<String>,
    price: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoinbaseTicker {
    price: Option<String>,
    amount: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KrakenResponse {
    result: Option<std::collections::HashMap<String, KrakenPair>>,
}

#[derive(Debug, Deserialize)]
struct KrakenPair {
    c: Option<Vec<String>>, // last trade closed [price, lot-volume]
}

// ── Service ──────────────────────────────────────────────

pub struct OracleService {
    pool: PgPool,
    market_service: Arc<MarketService>,
    http: reqwest::Client,
    redis_url: String,
}

impl OracleService {
    pub fn new(pool: PgPool, market_service: Arc<MarketService>, redis_url: &str) -> Self {
        Self {
            pool,
            market_service,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            redis_url: redis_url.to_string(),
        }
    }

    // ── Read API (unchanged public interface) ─────────

    pub async fn get_price(&self, market_id: &Uuid) -> Option<MarketPrice> {
        // Try Redis cache first
        if let Some(cached) = self.get_cached_price(market_id).await {
            return Some(cached);
        }
        // Fallback to Postgres
        sqlx::query_as::<_, MarketPrice>("SELECT * FROM market_prices WHERE market_id = $1")
            .bind(market_id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
    }

    pub async fn get_all_prices(&self) -> Vec<MarketPrice> {
        sqlx::query_as::<_, MarketPrice>("SELECT * FROM market_prices ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
    }

    pub async fn get_price_value(&self, market_id: &Uuid) -> Option<i64> {
        self.get_price(market_id).await.map(|p| p.price)
    }

    /// TWAP price over the given window (in seconds).
    pub async fn get_twap(&self, market_id: &Uuid, window_secs: i64) -> Option<TwapPrice> {
        let cutoff = Utc::now() - chrono::Duration::seconds(window_secs);
        let row = sqlx::query_as::<_, (f64, i64)>(
            "SELECT AVG(price)::float8 AS twap, COUNT(*) AS samples
             FROM oracle_price_history
             WHERE market_id = $1 AND fetched_at >= $2",
        )
        .bind(market_id)
        .bind(cutoff)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()?;

        if row.1 == 0 {
            return None;
        }

        Some(TwapPrice {
            market_id: *market_id,
            twap: row.0,
            samples: row.1,
            window_secs,
        })
    }

    // ── Redis cache ───────────────────────────────────

    async fn get_cached_price(&self, market_id: &Uuid) -> Option<MarketPrice> {
        let client = redis::Client::open(self.redis_url.as_str()).ok()?;
        let mut conn = client.get_multiplexed_async_connection().await.ok()?;
        let key = format!("oracle:price:{market_id}");
        let raw: Option<String> = conn.get(&key).await.ok().flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn cache_price(&self, price: &MarketPrice) {
        let Ok(client) = redis::Client::open(self.redis_url.as_str()) else { return };
        let Ok(mut conn) = client.get_multiplexed_async_connection().await else { return };
        let key = format!("oracle:price:{}", price.market_id);
        if let Ok(json) = serde_json::to_string(price) {
            let _: Result<(), _> = conn.set_ex(&key, json, 10).await; // 10s TTL
        }
    }

    async fn publish_price_update(&self, price: &MarketPrice) {
        let Ok(client) = redis::Client::open(self.redis_url.as_str()) else { return };
        let Ok(mut conn) = client.get_multiplexed_async_connection().await else { return };
        if let Ok(json) = serde_json::to_string(price) {
            let _: Result<(), _> = conn.publish("oracle:price_updates", json).await;
        }
    }

    // ── External price fetching ───────────────────────

    fn symbol_for_market(market: &Market) -> String {
        let base = format!("{:?}", market.base_asset);
        let quote = format!("{:?}", market.quote_asset);
        format!("{base}{quote}")
    }

    async fn fetch_binance(&self, symbol: &str) -> Option<f64> {
        let url = format!(
            "https://api.binance.com/api/v3/ticker/price?symbol={}",
            symbol.to_uppercase()
        );
        let resp: BinanceTicker = self.http.get(&url).send().await.ok()?.json().await.ok()?;
        resp.price
            .or(resp.last_price)
            .and_then(|s| s.parse::<f64>().ok())
    }

    async fn fetch_coinbase(&self, symbol: &str) -> Option<f64> {
        // Coinbase uses dash-separated pairs: BTC-USD
        let base = &symbol[..3];
        let quote = &symbol[3..];
        let pair = format!("{}-{}", base.to_uppercase(), quote.to_uppercase());
        let url = format!("https://api.coinbase.com/v2/prices/{}/spot", pair);
        let resp: serde_json::Value = self.http.get(&url).send().await.ok()?.json().await.ok()?;
        resp["data"]["amount"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
    }

    async fn fetch_kraken(&self, symbol: &str) -> Option<f64> {
        // Kraken uses its own pair naming, try common mappings
        let upper = symbol.to_uppercase();
        let pair = match upper.as_str() {
            "ETHUSDC" | "ETHUSD" => "XETHZUSD",
            "BTCUSDC" | "BTCUSD" => "XXBTZUSD",
            "SOLUSDC" | "SOLUSD" => "SOLUSD",
            _ => &upper,
        };
        let url = format!("https://api.kraken.com/0/public/Ticker?pair={}", pair);
        let resp: KrakenResponse = self.http.get(&url).send().await.ok()?.json().await.ok()?;
        resp.result?
            .values()
            .next()?
            .c.as_ref()?
            .first()?
            .parse::<f64>()
            .ok()
    }

    /// Fetch from all sources in parallel, return source-tagged prices.
    async fn fetch_all_sources(&self, market: &Market) -> Vec<SourcePrice> {
        let symbol = Self::symbol_for_market(market);

        let (binance, coinbase, kraken) = tokio::join!(
            self.fetch_binance(&symbol),
            self.fetch_coinbase(&symbol),
            self.fetch_kraken(&symbol),
        );

        let mut prices = Vec::with_capacity(3);
        if let Some(p) = binance {
            prices.push(SourcePrice { source: "binance".into(), price: p });
        }
        if let Some(p) = coinbase {
            prices.push(SourcePrice { source: "coinbase".into(), price: p });
        }
        if let Some(p) = kraken {
            prices.push(SourcePrice { source: "kraken".into(), price: p });
        }
        prices
    }

    /// Compute the median of a slice of prices.
    fn median(prices: &mut [f64]) -> f64 {
        prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = prices.len();
        if n == 0 {
            return 0.0;
        }
        if n % 2 == 0 {
            (prices[n / 2 - 1] + prices[n / 2]) / 2.0
        } else {
            prices[n / 2]
        }
    }

    /// Detect anomalies: if max deviation between any two sources exceeds threshold.
    fn detect_anomaly(
        market_id: Uuid,
        source_prices: &[SourcePrice],
    ) -> Option<PriceAnomaly> {
        if source_prices.len() < 2 {
            return None;
        }

        let prices: Vec<f64> = source_prices.iter().map(|s| s.price).collect();
        let min = prices.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        if min <= 0.0 {
            return None;
        }

        let deviation = (max - min) / min;
        if deviation > ANOMALY_THRESHOLD {
            Some(PriceAnomaly {
                market_id,
                prices: source_prices.to_vec(),
                max_deviation: deviation,
                detected_at: Utc::now(),
            })
        } else {
            None
        }
    }

    // ── Price update loop ─────────────────────────────

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
            let source_prices = self.fetch_all_sources(market).await;

            if source_prices.is_empty() {
                tracing::warn!(market_id = %market.id, "no_oracle_sources_responded");
                continue;
            }

            // Detect anomalies
            if let Some(anomaly) = Self::detect_anomaly(market.id, &source_prices) {
                tracing::warn!(
                    market_id = %anomaly.market_id,
                    deviation = anomaly.max_deviation,
                    sources = ?anomaly.prices,
                    "oracle_price_anomaly_detected"
                );
                // Still use the median, but log the anomaly
            }

            // Compute median price
            let mut raw: Vec<f64> = source_prices.iter().map(|s| s.price).collect();
            let median_f64 = Self::median(&mut raw);
            let price = (median_f64 * 100.0) as i64; // convert to cents

            let sources_str = source_prices
                .iter()
                .map(|s| s.source.as_str())
                .collect::<Vec<_>>()
                .join(",");

            // Upsert into market_prices
            if let Err(e) = sqlx::query(
                "INSERT INTO market_prices (market_id, price, source, updated_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (market_id) DO UPDATE SET
                    price = EXCLUDED.price,
                    source = EXCLUDED.source,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(market.id)
            .bind(price)
            .bind(&sources_str)
            .bind(now)
            .execute(&self.pool)
            .await
            {
                tracing::error!(market_id = %market.id, error = %e, "oracle_price_upsert_failed");
                continue;
            }

            // Insert into price history
            if let Err(e) = sqlx::query(
                "INSERT INTO oracle_price_history (market_id, price, source, fetched_at)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(market.id)
            .bind(price)
            .bind(&sources_str)
            .bind(now)
            .execute(&self.pool)
            .await
            {
                tracing::warn!(market_id = %market.id, error = %e, "oracle_history_insert_failed");
            }

            // Cache in Redis + publish update
            let mp = MarketPrice {
                market_id: market.id,
                price,
                source: sources_str,
                updated_at: now,
            };
            self.cache_price(&mp).await;
            self.publish_price_update(&mp).await;

            tracing::debug!(
                market_id = %market.id,
                price,
                sources = source_prices.len(),
                "oracle_price_updated"
            );
        }

        // Prune old history periodically (every ~100th iteration is fine, do it cheaply)
        let _ = sqlx::query(
            "DELETE FROM oracle_price_history
             WHERE id IN (
                SELECT id FROM oracle_price_history
                WHERE fetched_at < NOW() - INTERVAL '24 hours'
                ORDER BY fetched_at ASC
                LIMIT 500
             )",
        )
        .execute(&self.pool)
        .await;
    }

    /// Background worker that fetches prices every UPDATE_INTERVAL_SECS.
    pub async fn run_price_feed(self: Arc<Self>, cancel: CancellationToken) {
        tracing::info!(
            interval_secs = UPDATE_INTERVAL_SECS,
            sources = "binance,coinbase,kraken",
            "Oracle price feed started"
        );

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

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd() {
        assert_eq!(OracleService::median(&mut [3.0, 1.0, 2.0]), 2.0);
    }

    #[test]
    fn median_even() {
        assert_eq!(OracleService::median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn median_single() {
        assert_eq!(OracleService::median(&mut [42.0]), 42.0);
    }

    #[test]
    fn median_empty() {
        assert_eq!(OracleService::median(&mut []), 0.0);
    }

    #[test]
    fn anomaly_detected() {
        let prices = vec![
            SourcePrice { source: "binance".into(), price: 3500.0 },
            SourcePrice { source: "coinbase".into(), price: 5500.0 },
        ];
        let anomaly = OracleService::detect_anomaly(Uuid::new_v4(), &prices);
        assert!(anomaly.is_some());
        let a = anomaly.unwrap();
        assert!(a.max_deviation > ANOMALY_THRESHOLD);
    }

    #[test]
    fn no_anomaly_for_close_prices() {
        let prices = vec![
            SourcePrice { source: "binance".into(), price: 3500.0 },
            SourcePrice { source: "coinbase".into(), price: 3520.0 },
            SourcePrice { source: "kraken".into(), price: 3490.0 },
        ];
        let anomaly = OracleService::detect_anomaly(Uuid::new_v4(), &prices);
        assert!(anomaly.is_none());
    }

    #[test]
    fn no_anomaly_single_source() {
        let prices = vec![
            SourcePrice { source: "binance".into(), price: 3500.0 },
        ];
        assert!(OracleService::detect_anomaly(Uuid::new_v4(), &prices).is_none());
    }

    #[test]
    fn median_with_outlier() {
        // Median is robust to outliers
        let mut prices = vec![3500.0, 3510.0, 9999.0];
        assert_eq!(OracleService::median(&mut prices), 3510.0);
    }
}
