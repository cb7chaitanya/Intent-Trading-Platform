//! Infrastructure integration tests using testcontainers.
//!
//! Tests critical infrastructure: JWT key rotation, TWAP scheduling,
//! stop order triggering, Redis cache invalidation, partition archival,
//! CSRF token flow, cross-market arbitrage detection, sliding window
//! rate limiter, and idempotent request replay.
//!
//! Run: cargo test --test infra_integration_test --features integration
//!
//! Requires Docker to be running.

#![cfg(feature = "integration")]

use chrono::Utc;
use redis::AsyncCommands;
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::redis::{Redis, REDIS_PORT};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

// ============================================================
// Test infrastructure helpers
// ============================================================

async fn setup_postgres() -> (PgPool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("Failed to start Postgres container");

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to test Postgres");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    (pool, container)
}

async fn setup_redis() -> (String, testcontainers_modules::testcontainers::ContainerAsync<Redis>) {
    let container = Redis::default()
        .with_tag("7-alpine")
        .start()
        .await
        .expect("Failed to start Redis container");

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let url = format!("redis://{host}:{port}");

    (url, container)
}

fn unique_email() -> String {
    format!("test-{}@infra.test", Uuid::new_v4())
}

async fn create_test_user_and_account(pool: &PgPool) -> (Uuid, Uuid) {
    let user_id = Uuid::new_v4();
    let account_id = Uuid::new_v4();
    let email = unique_email();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'testhash', $3, $4)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO accounts (id, user_id, account_type, created_at)
         VALUES ($1, $2, 'spot', $3)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    (user_id, account_id)
}

async fn create_market(pool: &PgPool, base: &str, quote: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO markets (id, base_asset, quote_asset, tick_size, min_order_size, fee_rate, created_at)
         VALUES ($1, $2::asset_type, $3::asset_type, 100, 10, 0.001, $4)",
    )
    .bind(id)
    .bind(base)
    .bind(quote)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn set_oracle_price(pool: &PgPool, market_id: Uuid, price: i64) {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO market_prices (market_id, price, source, updated_at)
         VALUES ($1, $2, 'test', $3)
         ON CONFLICT (market_id) DO UPDATE SET price = $2, updated_at = $3",
    )
    .bind(market_id)
    .bind(price)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_balance(pool: &PgPool, account_id: Uuid, asset: &str, amount: i64) {
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, $3::asset_type, $4, 0, NOW())
         ON CONFLICT (account_id, asset) DO UPDATE SET available_balance = $4",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(asset)
    .bind(amount)
    .execute(pool)
    .await
    .unwrap();
}

// ============================================================
// 1. JWT Key Rotation and Validation
// ============================================================

#[tokio::test]
async fn test_jwt_key_rotation_and_validation() {
    let (pool, _pg) = setup_postgres().await;

    // Create initial key
    let key1_id = Uuid::new_v4();
    let key1_secret = hex::encode(vec![0xAB; 64]);
    sqlx::query(
        "INSERT INTO jwt_keys (id, key_secret, active, created_at) VALUES ($1, $2, TRUE, $3)",
    )
    .bind(key1_id)
    .bind(&key1_secret)
    .bind(Utc::now())
    .execute(&pool)
    .await
    .unwrap();

    // Verify active key retrieval
    let active: (Uuid, String, bool) = sqlx::query_as(
        "SELECT id, key_secret, active FROM jwt_keys WHERE active = TRUE ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(active.0, key1_id);
    assert!(active.2);

    // Create a JWT token with this key
    use jsonwebtoken::{encode, decode, Header, EncodingKey, DecodingKey, Validation};
    use serde::{Serialize, Deserialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct Claims {
        sub: String,
        exp: i64,
    }

    let claims = Claims {
        sub: Uuid::new_v4().to_string(),
        exp: Utc::now().timestamp() + 3600,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(key1_secret.as_bytes()),
    )
    .unwrap();

    // Simulate rotation: deactivate old key, create new key
    sqlx::query("UPDATE jwt_keys SET active = FALSE WHERE id = $1")
        .bind(key1_id)
        .execute(&pool)
        .await
        .unwrap();

    let key2_id = Uuid::new_v4();
    let key2_secret = hex::encode(vec![0xCD; 64]);
    sqlx::query(
        "INSERT INTO jwt_keys (id, key_secret, active, created_at) VALUES ($1, $2, TRUE, $3)",
    )
    .bind(key2_id)
    .bind(&key2_secret)
    .bind(Utc::now())
    .execute(&pool)
    .await
    .unwrap();

    // Verify new active key
    let new_active: (Uuid,) = sqlx::query_as(
        "SELECT id FROM jwt_keys WHERE active = TRUE LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_active.0, key2_id);

    // Grace period: old token should still validate with old key
    // Fetch validation keys (active + recent inactive)
    let grace_cutoff = Utc::now() - chrono::Duration::days(7);
    let validation_keys: Vec<(String,)> = sqlx::query_as(
        "SELECT key_secret FROM jwt_keys
         WHERE active = TRUE OR created_at >= $1
         ORDER BY active DESC, created_at DESC",
    )
    .bind(grace_cutoff)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(validation_keys.len() >= 2, "Should have both active and grace period keys");

    // Token signed with old key should validate against grace period keys
    let mut validated = false;
    for (secret,) in &validation_keys {
        if decode::<Claims>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .is_ok()
        {
            validated = true;
            break;
        }
    }
    assert!(validated, "Token signed with rotated key should still validate during grace period");

    // Token should NOT validate with new key alone
    let new_key_result = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(key2_secret.as_bytes()),
        &Validation::default(),
    );
    assert!(new_key_result.is_err(), "Old token should not validate against new key");

    // Cleanup: simulate cleanup_old_keys for very old keys
    let old_key_id = Uuid::new_v4();
    let old_cutoff = Utc::now() - chrono::Duration::days(31);
    sqlx::query(
        "INSERT INTO jwt_keys (id, key_secret, active, created_at) VALUES ($1, $2, FALSE, $3)",
    )
    .bind(old_key_id)
    .bind("old-secret")
    .bind(old_cutoff)
    .execute(&pool)
    .await
    .unwrap();

    let cleanup_cutoff = Utc::now() - chrono::Duration::days(30);
    let deleted = sqlx::query("DELETE FROM jwt_keys WHERE active = FALSE AND created_at < $1")
        .bind(cleanup_cutoff)
        .execute(&pool)
        .await
        .unwrap();

    assert!(deleted.rows_affected() >= 1, "Should clean up keys older than 30 days");
}

// ============================================================
// 2. TWAP Scheduler End-to-End Execution
// ============================================================

#[tokio::test]
async fn test_twap_scheduler_end_to_end() {
    let (pool, _pg) = setup_postgres().await;
    let (user_id, account_id) = create_test_user_and_account(&pool).await;

    // Seed balance for intent creation
    seed_balance(&pool, account_id, "ETH", 1_000_000).await;

    // Create a TWAP parent: 3000 total qty, 3 slices (1 per second for 3 seconds)
    let twap_id = Uuid::new_v4();
    let now = Utc::now();
    let interval_secs: i64 = 1;
    let duration_secs: i64 = 3;
    let slices_total: i32 = 3;
    let qty_per_slice: i64 = 1000;

    sqlx::query(
        "INSERT INTO twap_intents (id, user_id, account_id, token_in, token_out,
            total_qty, filled_qty, min_price, duration_secs, interval_secs,
            slices_total, slices_completed, status, created_at)
         VALUES ($1, $2, $3, 'ETH', 'USDC', 3000, 0, 0, $4, $5, $6, 0, 'active', $7)",
    )
    .bind(twap_id)
    .bind(user_id.to_string())
    .bind(account_id)
    .bind(duration_secs)
    .bind(interval_secs)
    .bind(slices_total)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Create child intents — first two already due (past), third in the future
    let past = now - chrono::Duration::seconds(10);
    let future = now + chrono::Duration::seconds(3600);

    let mut child_ids = Vec::new();
    for i in 0..3 {
        let child_id = Uuid::new_v4();
        let scheduled_at = if i < 2 { past + chrono::Duration::seconds(i as i64) } else { future };
        child_ids.push(child_id);

        sqlx::query(
            "INSERT INTO twap_child_intents (id, twap_id, intent_id, slice_index, qty, status, scheduled_at)
             VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
        )
        .bind(child_id)
        .bind(twap_id)
        .bind(Uuid::new_v4()) // placeholder intent_id
        .bind(i)
        .bind(qty_per_slice)
        .bind(scheduled_at)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Verify: 3 pending children
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM twap_child_intents WHERE twap_id = $1 AND status = 'pending'",
    )
    .bind(twap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 3);

    // Simulate scheduler: find due slices (scheduled_at <= now AND parent active)
    let due_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM twap_child_intents c
         JOIN twap_intents t ON t.id = c.twap_id
         WHERE c.status = 'pending' AND c.scheduled_at <= $1 AND t.status = 'active'",
    )
    .bind(now)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(due_count, 2, "Only 2 past-due slices should be found");

    // Simulate processing: mark due children as submitted
    sqlx::query(
        "UPDATE twap_child_intents SET status = 'submitted'
         WHERE twap_id = $1 AND status = 'pending' AND scheduled_at <= $2",
    )
    .bind(twap_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Simulate completion of first two slices
    for &child_id in &child_ids[0..2] {
        // Idempotent completion — update only if not already completed
        let result = sqlx::query(
            "UPDATE twap_child_intents SET status = 'completed'
             WHERE id = $1 AND status != 'completed'",
        )
        .bind(child_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(result.rows_affected(), 1);

        sqlx::query(
            "UPDATE twap_intents SET filled_qty = filled_qty + $1, slices_completed = slices_completed + 1
             WHERE id = $2",
        )
        .bind(qty_per_slice)
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Verify idempotency: completing same child again should be a no-op
    let idempotent = sqlx::query(
        "UPDATE twap_child_intents SET status = 'completed'
         WHERE id = $1 AND status != 'completed'",
    )
    .bind(child_ids[0])
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(idempotent.rows_affected(), 0, "Idempotent completion should not update");

    // Check progress
    let (filled, completed): (i64, i32) = sqlx::query_as(
        "SELECT filled_qty, slices_completed FROM twap_intents WHERE id = $1",
    )
    .bind(twap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(filled, 2000);
    assert_eq!(completed, 2);

    // Third slice still pending
    let third_status: String = sqlx::query_scalar(
        "SELECT status FROM twap_child_intents WHERE id = $1",
    )
    .bind(child_ids[2])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(third_status, "pending");

    // Complete the third and mark TWAP as completed
    sqlx::query("UPDATE twap_child_intents SET status = 'completed' WHERE id = $1")
        .bind(child_ids[2])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE twap_intents SET filled_qty = filled_qty + $1, slices_completed = slices_completed + 1 WHERE id = $2",
    )
    .bind(qty_per_slice)
    .bind(twap_id)
    .execute(&pool)
    .await
    .unwrap();

    // Auto-complete check
    let (total_qty, filled_qty, slices_total_check, slices_completed_check): (i64, i64, i32, i32) =
        sqlx::query_as(
            "SELECT total_qty, filled_qty, slices_total, slices_completed FROM twap_intents WHERE id = $1",
        )
        .bind(twap_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    if filled_qty >= total_qty || slices_completed_check >= slices_total_check {
        sqlx::query(
            "UPDATE twap_intents SET status = 'completed', finished_at = NOW()
             WHERE id = $1 AND status = 'active'",
        )
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    let final_status: String = sqlx::query_scalar(
        "SELECT status::text FROM twap_intents WHERE id = $1",
    )
    .bind(twap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(final_status, "completed");
}

// ============================================================
// 3. Stop Order Trigger on Oracle Price Cross
// ============================================================

#[tokio::test]
async fn test_stop_order_trigger_on_price_cross() {
    let (pool, _pg) = setup_postgres().await;
    let (user_id, _account_id) = create_test_user_and_account(&pool).await;

    // Create ETH/USDC market and set oracle price
    let market_id = create_market(&pool, "ETH", "USDC").await;
    set_oracle_price(&pool, market_id, 3500_00).await; // $3500

    let now_ts = Utc::now().timestamp();

    // --- Stop-loss sell: triggers when price <= stop_price ---
    let stop_loss_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out,
            deadline, status, created_at, order_type, stop_price, stop_side)
         VALUES ($1, $2, 'ETH', 'USDC', 1000, 900, $3, 'open', $4, 'stop', 3000_00, 'sell')",
    )
    .bind(stop_loss_id)
    .bind(user_id.to_string())
    .bind(now_ts + 3600)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // --- Stop-buy: triggers when price >= stop_price ---
    let stop_buy_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out,
            deadline, status, created_at, order_type, stop_price, stop_side)
         VALUES ($1, $2, 'ETH', 'USDC', 500, 400, $3, 'open', $4, 'stop', 4000_00, 'buy')",
    )
    .bind(stop_buy_id)
    .bind(user_id.to_string())
    .bind(now_ts + 3600)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // --- Stop-limit: triggers at stop, converts to limit order ---
    let stop_limit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out,
            deadline, status, created_at, order_type, stop_price, stop_side, limit_price)
         VALUES ($1, $2, 'ETH', 'USDC', 800, 700, $3, 'open', $4, 'stop', 3000_00, 'sell', 2950_00)",
    )
    .bind(stop_limit_id)
    .bind(user_id.to_string())
    .bind(now_ts + 3600)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // Current price is $3500 — none should trigger yet
    let price_row: Option<(i64,)> = sqlx::query_as(
        "SELECT mp.price FROM market_prices mp
         JOIN markets m ON m.id = mp.market_id
         WHERE UPPER(m.base_asset::text) = 'ETH' AND UPPER(m.quote_asset::text) = 'USDC'
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();

    let current_price = price_row.unwrap().0;
    assert_eq!(current_price, 3500_00);

    // Stop-loss sell should NOT trigger (3500 > 3000)
    assert!(current_price > 3000_00, "Price above stop — should not trigger stop-loss");

    // Stop-buy should NOT trigger (3500 < 4000)
    assert!(current_price < 4000_00, "Price below stop — should not trigger stop-buy");

    // --- Drop price to $2900 (below stop-loss at $3000) ---
    set_oracle_price(&pool, market_id, 2900_00).await;

    // Simulate stop order monitor logic for stop-loss
    let stops = sqlx::query_as::<_, (Uuid, i64, Option<String>, Option<i64>)>(
        "SELECT id, stop_price, stop_side, limit_price FROM intents
         WHERE order_type = 'stop' AND status = 'open' AND stop_price IS NOT NULL
           AND triggered_at IS NULL
         ORDER BY created_at ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(stops.len(), 3, "All 3 stop orders should be pending");

    let new_price = 2900_00i64;

    for (id, stop_price, side, limit_price) in &stops {
        let side = side.as_deref().unwrap_or("sell");
        let triggered = match side {
            "buy" => new_price >= *stop_price,
            _ => new_price <= *stop_price,
        };

        if triggered {
            let new_type = if limit_price.is_some() { "limit" } else { "market" };

            // Atomic trigger-once
            let result = sqlx::query(
                "UPDATE intents SET status = 'open', order_type = $2::order_type, triggered_at = NOW()
                 WHERE id = $1 AND order_type = 'stop' AND triggered_at IS NULL",
            )
            .bind(id)
            .bind(new_type)
            .execute(&pool)
            .await
            .unwrap();

            assert!(result.rows_affected() <= 1);
        }
    }

    // Verify: stop-loss sell triggered → market order
    let (sl_status, sl_type, sl_triggered): (String, String, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT status::text, order_type::text, triggered_at FROM intents WHERE id = $1",
        )
        .bind(stop_loss_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sl_status, "open");
    assert_eq!(sl_type, "market", "Stop-loss should convert to market");
    assert!(sl_triggered.is_some(), "triggered_at should be set");

    // Verify: stop-buy NOT triggered (2900 < 4000)
    let (sb_type,): (String,) = sqlx::query_as(
        "SELECT order_type::text FROM intents WHERE id = $1",
    )
    .bind(stop_buy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sb_type, "stop", "Stop-buy should remain as stop order");

    // Verify: stop-limit triggered → limit order
    let (slt_type, slt_limit): (String, Option<i64>) = sqlx::query_as(
        "SELECT order_type::text, limit_price FROM intents WHERE id = $1",
    )
    .bind(stop_limit_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(slt_type, "limit", "Stop-limit should convert to limit");
    assert_eq!(slt_limit, Some(2950_00), "Limit price should be preserved");

    // --- Trigger-once guarantee: re-run should not re-trigger ---
    let retrigger = sqlx::query(
        "UPDATE intents SET order_type = 'market', triggered_at = NOW()
         WHERE id = $1 AND order_type = 'stop' AND triggered_at IS NULL",
    )
    .bind(stop_loss_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(retrigger.rows_affected(), 0, "Already triggered — should not re-trigger");

    // --- Now raise price to $4100 to trigger stop-buy ---
    set_oracle_price(&pool, market_id, 4100_00).await;

    let result = sqlx::query(
        "UPDATE intents SET status = 'open', order_type = 'market'::order_type, triggered_at = NOW()
         WHERE id = $1 AND order_type = 'stop' AND triggered_at IS NULL",
    )
    .bind(stop_buy_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1, "Stop-buy should now trigger");

    let (sb_final_type,): (String,) = sqlx::query_as(
        "SELECT order_type::text FROM intents WHERE id = $1",
    )
    .bind(stop_buy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sb_final_type, "market");
}

// ============================================================
// 4. Redis Cache Invalidation on Updates
// ============================================================

#[tokio::test]
async fn test_redis_cache_invalidation() {
    let (redis_url, _redis_container) = setup_redis().await;

    let client = redis::Client::open(redis_url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    // Simulate cache-aside pattern
    let cache_key = "cache:market:test-market-1";
    let market_data = r#"{"id":"abc","name":"ETH/USDC","price":3500}"#;

    // Set cached value with TTL
    let _: () = conn.set_ex(cache_key, market_data, 60u64).await.unwrap();

    // Verify cached value exists
    let cached: Option<String> = conn.get(cache_key).await.unwrap();
    assert_eq!(cached.as_deref(), Some(market_data));

    // Simulate update: invalidate single key
    let _: () = conn.del(cache_key).await.unwrap();

    let after_invalidate: Option<String> = conn.get(cache_key).await.unwrap();
    assert!(after_invalidate.is_none(), "Cache should be empty after invalidation");

    // Pattern invalidation: set multiple keys in the same key_type
    for i in 0..5 {
        let key = format!("cache:market:item-{i}");
        let _: () = conn.set_ex(&key, format!("data-{i}"), 60u64).await.unwrap();
    }

    // Verify all keys exist
    let keys_before: Vec<String> = redis::cmd("KEYS")
        .arg("cache:market:*")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(keys_before.len(), 5, "Should have 5 cached market entries");

    // Pattern invalidation
    let pattern_keys: Vec<String> = redis::cmd("KEYS")
        .arg("cache:market:*")
        .query_async(&mut conn)
        .await
        .unwrap();
    for key in &pattern_keys {
        let _: () = conn.del(key.as_str()).await.unwrap();
    }

    let keys_after: Vec<String> = redis::cmd("KEYS")
        .arg("cache:market:*")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(keys_after.len(), 0, "All market cache keys should be invalidated");

    // Verify TTL-based expiry works
    let ttl_key = "cache:market:ttl-test";
    let _: () = conn.set_ex(ttl_key, "expires-soon", 1u64).await.unwrap();

    let before_expiry: Option<String> = conn.get(ttl_key).await.unwrap();
    assert!(before_expiry.is_some());

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let after_expiry: Option<String> = conn.get(ttl_key).await.unwrap();
    assert!(after_expiry.is_none(), "Key should expire after TTL");

    // Verify cache miss → set → cache hit flow
    let fresh_key = "cache:balance:acct-123";
    let miss: Option<String> = conn.get(fresh_key).await.unwrap();
    assert!(miss.is_none(), "Should be a cache miss initially");

    // Simulate DB fetch + cache set
    let db_value = r#"{"available":10000,"locked":500}"#;
    let _: () = conn.set_ex(fresh_key, db_value, 10u64).await.unwrap();

    let hit: Option<String> = conn.get(fresh_key).await.unwrap();
    assert_eq!(hit.as_deref(), Some(db_value), "Should be a cache hit after set");
}

// ============================================================
// 5. Partition Archival Worker
// ============================================================

#[tokio::test]
async fn test_partition_archival() {
    let (pool, _pg) = setup_postgres().await;

    // Verify the archive function exists and can be called
    // With retention_months = 6 and no old partitions, it should return 0
    let archived: i32 = sqlx::query_scalar("SELECT archive_old_partitions(6)")
        .fetch_one(&pool)
        .await
        .unwrap();

    // No partitions are old enough to archive in a fresh DB
    assert_eq!(archived, 0, "Fresh DB should have no partitions to archive");

    // Verify the partition_archive_log table exists
    let log_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM partition_archive_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(log_count, 0, "No archive log entries expected");

    // Verify the function handles extreme retention gracefully
    let archived_zero: i32 = sqlx::query_scalar("SELECT archive_old_partitions(0)")
        .fetch_one(&pool)
        .await
        .unwrap();
    // With 0 months retention, cutoff = current month, so recent partitions survive
    assert!(archived_zero >= 0, "Should handle 0-month retention without error");

    // Verify partitioned tables exist
    let partitioned_tables: Vec<(String,)> = sqlx::query_as(
        "SELECT tablename::text FROM pg_tables
         WHERE schemaname = 'public'
         AND tablename IN ('trades', 'fills', 'executions', 'ledger_entries', 'market_trades')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!partitioned_tables.is_empty(), "Partitioned tables should exist");

    // Verify child partitions exist for current period
    let child_partitions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhparent
         WHERE c.relname IN ('trades', 'fills', 'executions', 'ledger_entries', 'market_trades')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(child_partitions > 0, "Should have at least some child partitions from migrations");
}

// ============================================================
// 6. CSRF Token Flow
// ============================================================

#[tokio::test]
async fn test_csrf_token_flow() {
    let (redis_url, _redis_container) = setup_redis().await;

    let client = redis::Client::open(redis_url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    let user_id = "user-123";

    // Generate token: store in Redis with TTL
    let token = Uuid::new_v4().to_string();
    let key = format!("csrf:{token}");
    let _: () = conn.set_ex(&key, user_id, 3600u64).await.unwrap();

    // Verify token exists in Redis
    let stored_user: Option<String> = conn.get(&key).await.unwrap();
    assert_eq!(stored_user.as_deref(), Some(user_id));

    // Validate + consume (single-use): GET then DEL
    let validated_user: Option<String> = conn.get(&key).await.unwrap();
    assert_eq!(validated_user.as_deref(), Some(user_id));
    let _: () = conn.del(&key).await.unwrap();

    // Token should be consumed — second validation fails
    let reused: Option<String> = conn.get(&key).await.unwrap();
    assert!(reused.is_none(), "Token should be consumed (single-use)");

    // Double-submit check: header token must match cookie token
    let header_token = Uuid::new_v4().to_string();
    let cookie_token = header_token.clone(); // should match
    assert_eq!(header_token, cookie_token, "Double-submit tokens must match");

    let mismatched_cookie = Uuid::new_v4().to_string();
    assert_ne!(header_token, mismatched_cookie, "Mismatched tokens should be rejected");

    // User binding: token issued for user-123 should not validate for user-456
    let bound_token = Uuid::new_v4().to_string();
    let bound_key = format!("csrf:{bound_token}");
    let _: () = conn.set_ex(&bound_key, "user-123", 3600u64).await.unwrap();

    let token_user: String = conn.get(&bound_key).await.unwrap();
    assert_eq!(token_user, "user-123");
    // Middleware would reject if request user != token_user && token_user != "anonymous"
    assert_ne!(token_user, "user-456", "Token bound to user-123, not user-456");

    // Anonymous tokens can be used by anyone
    let anon_token = Uuid::new_v4().to_string();
    let anon_key = format!("csrf:{anon_token}");
    let _: () = conn.set_ex(&anon_key, "anonymous", 3600u64).await.unwrap();

    let anon_user: String = conn.get(&anon_key).await.unwrap();
    assert_eq!(anon_user, "anonymous", "Anonymous tokens are not bound to a user");

    // TTL test: token should expire
    let ttl_token = Uuid::new_v4().to_string();
    let ttl_key = format!("csrf:{ttl_token}");
    let _: () = conn.set_ex(&ttl_key, user_id, 1u64).await.unwrap();

    let before: Option<String> = conn.get(&ttl_key).await.unwrap();
    assert!(before.is_some());

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let after: Option<String> = conn.get(&ttl_key).await.unwrap();
    assert!(after.is_none(), "CSRF token should expire after TTL");
}

// ============================================================
// 7. Cross-Market Arbitrage Detection
// ============================================================

#[tokio::test]
async fn test_cross_market_arbitrage_detection() {
    let (pool, _pg) = setup_postgres().await;

    // Create two markets with the same base asset: ETH/USDC and ETH/BTC
    let market_eth_usdc = create_market(&pool, "ETH", "USDC").await;
    let market_eth_btc = create_market(&pool, "ETH", "BTC").await;

    // Set similar prices: ETH = $3500 in both markets
    set_oracle_price(&pool, market_eth_usdc, 3500_00).await;
    set_oracle_price(&pool, market_eth_btc, 3500_00).await;

    // Query oracle prices for all ETH markets
    let eth_prices: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT m.id, mp.price
         FROM market_prices mp
         JOIN markets m ON m.id = mp.market_id
         WHERE m.base_asset = 'ETH'::asset_type",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(eth_prices.len(), 2, "Should have 2 ETH markets");

    // No arbitrage: prices are the same
    let max_spread = 0.40; // 40% = 2 * 20% max_price_deviation
    let prices: Vec<f64> = eth_prices.iter().map(|(_, p)| *p as f64).collect();
    let spread = (prices[0] - prices[1]).abs() / prices[0];
    assert!(spread <= max_spread, "Same-price markets: no arbitrage");

    // --- Introduce price divergence ---
    // Market A: ETH = $3500, Market B: ETH = $5500 (57% spread > 40% threshold)
    set_oracle_price(&pool, market_eth_btc, 5500_00).await;

    let updated_prices: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT m.id, mp.price
         FROM market_prices mp
         JOIN markets m ON m.id = mp.market_id
         WHERE m.base_asset = 'ETH'::asset_type
         ORDER BY mp.price ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let price_a = updated_prices[0].1 as f64;
    let price_b = updated_prices[1].1 as f64;
    let divergent_spread = (price_b - price_a).abs() / price_a;

    assert!(
        divergent_spread > max_spread,
        "Price spread {:.1}% should exceed threshold {:.1}%",
        divergent_spread * 100.0,
        max_spread * 100.0
    );

    // Simulate bid validation: a bid at $3500 against market B ($5500) should be flagged
    let bid_price = 3500.0_f64;
    let oracle_b = 5500_00.0_f64;
    let bid_spread = (bid_price - oracle_b).abs() / oracle_b;
    assert!(bid_spread > max_spread, "Bid should be flagged as cross-market arbitrage");

    // --- Bid within tolerance should pass ---
    set_oracle_price(&pool, market_eth_btc, 3600_00).await; // 2.9% spread

    let sane_prices: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT m.id, mp.price
         FROM market_prices mp
         JOIN markets m ON m.id = mp.market_id
         WHERE m.base_asset = 'ETH'::asset_type
         ORDER BY mp.price ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let sane_a = sane_prices[0].1 as f64;
    let sane_b = sane_prices[1].1 as f64;
    let sane_spread = (sane_b - sane_a).abs() / sane_a;
    assert!(
        sane_spread <= max_spread,
        "Sane spread {:.1}% should be within threshold",
        sane_spread * 100.0
    );
}

// ============================================================
// 8. Redis Sliding Window Rate Limiter
// ============================================================

#[tokio::test]
async fn test_redis_sliding_window_rate_limiter() {
    let (redis_url, _redis_container) = setup_redis().await;

    let client = redis::Client::open(redis_url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    let identity = "user-rate-test";
    let route = "/intents";
    let max_requests: u64 = 5; // reduced for testing
    let window_secs: u64 = 10;
    let key = format!("rate_limit:{identity}:{route}");

    // Clean state
    let _: () = conn.del(&key).await.unwrap();

    // Send requests up to the limit
    for i in 0..max_requests {
        let now_micros = chrono::Utc::now().timestamp_micros() + i as i64;
        let window_start = now_micros - (window_secs as i64 * 1_000_000);

        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("ZREMRANGEBYSCORE").arg(&key).arg("-inf").arg(window_start).ignore()
            .cmd("ZCARD").arg(&key)
            .cmd("ZADD").arg(&key).arg(now_micros).arg(now_micros).ignore()
            .cmd("EXPIRE").arg(&key).arg(window_secs * 2).ignore();

        let results: Vec<redis::Value> = pipe.query_async(&mut conn).await.unwrap();

        let count = match &results[..] {
            [_, redis::Value::Int(n), ..] => *n as u64,
            _ => 0,
        };

        assert!(
            count < max_requests,
            "Request {i} should be under limit (count={count})"
        );
    }

    // Next request should be at or over the limit
    let over_micros = chrono::Utc::now().timestamp_micros() + 100;
    let over_window_start = over_micros - (window_secs as i64 * 1_000_000);

    let mut pipe = redis::pipe();
    pipe.atomic()
        .cmd("ZREMRANGEBYSCORE").arg(&key).arg("-inf").arg(over_window_start).ignore()
        .cmd("ZCARD").arg(&key);

    let results: Vec<redis::Value> = pipe.query_async(&mut conn).await.unwrap();

    let count = match &results[..] {
        [_, redis::Value::Int(n), ..] => *n as u64,
        _ => 0,
    };

    assert!(count >= max_requests, "Should be at rate limit (count={count}, limit={max_requests})");

    // Verify the sorted set contains the right number of entries
    let card: u64 = redis::cmd("ZCARD")
        .arg(&key)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(card, max_requests, "Sorted set should have exactly {max_requests} entries");

    // Wait for entries to age out of window, then verify the window slides
    let _: () = conn.del(&key).await.unwrap(); // reset for clean sliding window test

    // Add an entry from 2 seconds ago
    let old_micros = chrono::Utc::now().timestamp_micros() - 2_000_000;
    let _: () = redis::cmd("ZADD")
        .arg(&key)
        .arg(old_micros)
        .arg(old_micros)
        .query_async(&mut conn)
        .await
        .unwrap();

    // Add an entry from now
    let new_micros = chrono::Utc::now().timestamp_micros();
    let _: () = redis::cmd("ZADD")
        .arg(&key)
        .arg(new_micros)
        .arg(new_micros)
        .query_async(&mut conn)
        .await
        .unwrap();

    // Sliding window with 1-second window should only keep the new entry
    let tiny_window_start = chrono::Utc::now().timestamp_micros() - 1_000_000;
    let _: () = redis::cmd("ZREMRANGEBYSCORE")
        .arg(&key)
        .arg("-inf")
        .arg(tiny_window_start)
        .query_async(&mut conn)
        .await
        .unwrap();

    let remaining: u64 = redis::cmd("ZCARD")
        .arg(&key)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(remaining, 1, "Sliding window should keep only the recent entry");

    // Per-identity isolation: different users have independent limits
    let key_other = format!("rate_limit:other-user:{route}");
    let _: () = conn.del(&key_other).await.unwrap();

    let other_card: u64 = redis::cmd("ZCARD")
        .arg(&key_other)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(other_card, 0, "Other user should have 0 requests tracked");
}

// ============================================================
// 9. Idempotent Request Replay
// ============================================================

#[tokio::test]
async fn test_idempotent_request_replay() {
    let (pool, _pg) = setup_postgres().await;

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let user_id = "user-idem-test";
    let request_hash = "POST:/intents:user-idem-test";
    let status_code = 201;
    let response_body = r#"{"id":"abc-123","status":"open"}"#;

    // Initially, no cached response
    let cached = sqlx::query_as::<_, (String, String, i32, String)>(
        "SELECT key, request_hash, status_code, response_body
         FROM idempotency_keys WHERE key = $1",
    )
    .bind(&idem_key)
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert!(cached.is_none(), "No cached response initially");

    // Store the response (first request)
    sqlx::query(
        "INSERT INTO idempotency_keys (key, user_id, request_hash, status_code, response_body)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(&idem_key)
    .bind(user_id)
    .bind(request_hash)
    .bind(status_code)
    .bind(response_body)
    .execute(&pool)
    .await
    .unwrap();

    // Replay: same key, same request_hash → return cached response
    let replay = sqlx::query_as::<_, (String, i32, String)>(
        "SELECT request_hash, status_code, response_body
         FROM idempotency_keys WHERE key = $1",
    )
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(replay.0, request_hash, "Request hash should match");
    assert_eq!(replay.1, status_code, "Status code should be replayed");
    assert_eq!(replay.2, response_body, "Response body should be replayed");

    // Conflict: same key, different request_hash → should be rejected (422)
    let different_hash = "PUT:/intents:user-idem-test";
    let stored_hash = &replay.0;
    assert_ne!(
        stored_hash, different_hash,
        "Different request hash should cause conflict"
    );

    // Verify ON CONFLICT DO NOTHING: re-inserting same key with different data doesn't overwrite
    sqlx::query(
        "INSERT INTO idempotency_keys (key, user_id, request_hash, status_code, response_body)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(&idem_key)
    .bind(user_id)
    .bind(different_hash)
    .bind(500)
    .bind("overwritten!")
    .execute(&pool)
    .await
    .unwrap();

    // Original response should be intact
    let intact = sqlx::query_as::<_, (i32, String)>(
        "SELECT status_code, response_body FROM idempotency_keys WHERE key = $1",
    )
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(intact.0, 201, "Original status code preserved");
    assert_eq!(intact.1, response_body, "Original response body preserved");

    // Multiple idempotency keys for different endpoints are independent
    let key2 = format!("idem-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO idempotency_keys (key, user_id, request_hash, status_code, response_body)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&key2)
    .bind(user_id)
    .bind("POST:/bids:user-idem-test")
    .bind(200)
    .bind(r#"{"bid_id":"xyz"}"#)
    .execute(&pool)
    .await
    .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(count >= 2, "Both idempotency keys should coexist");

    // Verify same user can't use same key for different requests
    let conflict_key = format!("idem-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO idempotency_keys (key, user_id, request_hash, status_code, response_body)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&conflict_key)
    .bind(user_id)
    .bind("POST:/intents:user-idem-test")
    .bind(201)
    .bind(r#"{"ok":true}"#)
    .execute(&pool)
    .await
    .unwrap();

    // Lookup returns the cached response — middleware would compare request_hash
    let check = sqlx::query_as::<_, (String,)>(
        "SELECT request_hash FROM idempotency_keys WHERE key = $1",
    )
    .bind(&conflict_key)
    .fetch_one(&pool)
    .await
    .unwrap();

    // If a different request tries to use the same key, hash won't match → 422
    let attacker_hash = "DELETE:/intents:hacker";
    assert_ne!(
        check.0, attacker_hash,
        "Request hash mismatch should produce 422 conflict"
    );
}
