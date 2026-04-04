use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::Mutex;

use crate::metrics::counters;

/// TTL presets by data type (seconds).
pub struct CacheTtl;

impl CacheTtl {
    pub const MARKETS: u64 = 60;
    pub const MARKET_PRICE: u64 = 5;
    pub const BALANCES: u64 = 10;
    pub const ORDERBOOK: u64 = 3;
    pub const LEADERBOARD: u64 = 30;
}

#[derive(Clone)]
pub struct CacheService {
    conn: Arc<Mutex<MultiplexedConnection>>,
}

impl CacheService {
    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, key_type: &str, key: &str) -> Option<T> {
        let full_key = format!("cache:{key_type}:{key}");
        let mut conn = self.conn.lock().await;

        let raw: Option<String> = conn.get(&full_key).await.ok().flatten();

        match raw {
            Some(data) => {
                counters::CACHE_HITS.with_label_values(&[key_type]).inc();
                serde_json::from_str(&data).ok()
            }
            None => {
                counters::CACHE_MISSES.with_label_values(&[key_type]).inc();
                None
            }
        }
    }

    pub async fn set<T: Serialize>(&self, key_type: &str, key: &str, value: &T, ttl_secs: u64) {
        let full_key = format!("cache:{key_type}:{key}");
        let mut conn = self.conn.lock().await;

        if let Ok(json) = serde_json::to_string(value) {
            let _: Result<(), _> = conn.set_ex(&full_key, json, ttl_secs).await;
        }
    }

    pub async fn invalidate(&self, key_type: &str, key: &str) {
        let full_key = format!("cache:{key_type}:{key}");
        let mut conn = self.conn.lock().await;
        let _: Result<(), _> = conn.del(&full_key).await;
    }

    pub async fn invalidate_pattern(&self, key_type: &str) {
        let pattern = format!("cache:{key_type}:*");
        let mut conn = self.conn.lock().await;

        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut *conn)
            .await
            .unwrap_or_default();

        for key in &keys {
            let _: Result<(), _> = conn.del(key.as_str()).await;
        }
    }
}
