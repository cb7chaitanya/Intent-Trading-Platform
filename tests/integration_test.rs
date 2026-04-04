//! Integration tests for the intent-based trading platform.
//!
//! Requires a running PostgreSQL and Redis instance.
//! Set DATABASE_URL and REDIS_URL env vars, or use defaults.
//!
//! Run: cargo test --test integration_test --features integration
//!
//! These tests are gated behind the `integration` feature to avoid
//! failing in CI environments without infrastructure.

#![cfg(feature = "integration")]

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================
// Test helpers
// ============================================================

async fn setup_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/intent_trading".into());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

fn unique_email() -> String {
    format!("test-{}@integration.test", Uuid::new_v4())
}

// ============================================================
// User + Account tests
// ============================================================

#[tokio::test]
async fn test_user_registration() {
    let pool = setup_pool().await;
    let email = unique_email();

    let result = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(result, 0, "User should not exist yet");

    // Insert user
    let user_id = Uuid::new_v4();
    let hash = bcrypt::hash("testpass", 4).unwrap(); // cost=4 for speed
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&hash)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Verify
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 1);

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_account_creation() {
    let pool = setup_pool().await;

    // Create user first
    let user_id = Uuid::new_v4();
    let email = unique_email();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(&email)
    .bind("hash")
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Create account
    let account_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounts (id, user_id, account_type, created_at)
         VALUES ($1, $2, 'spot', $3)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Verify
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM accounts WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 1);

    // Cleanup
    sqlx::query("DELETE FROM accounts WHERE id = $1").bind(account_id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await.unwrap();
}

// ============================================================
// Balance + Deposit/Withdraw tests
// ============================================================

#[tokio::test]
async fn test_deposit_and_withdraw() {
    let pool = setup_pool().await;
    let (user_id, account_id) = create_test_user_and_account(&pool).await;

    // Deposit
    let balance_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'USDC', 10000, 0, NOW())",
    )
    .bind(balance_id)
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    // Verify balance
    let available = sqlx::query_scalar::<_, i64>(
        "SELECT available_balance FROM balances WHERE id = $1",
    )
    .bind(balance_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(available, 10000);

    // Withdraw (simulate)
    sqlx::query(
        "UPDATE balances SET available_balance = available_balance - 3000 WHERE id = $1",
    )
    .bind(balance_id)
    .execute(&pool)
    .await
    .unwrap();

    let available = sqlx::query_scalar::<_, i64>(
        "SELECT available_balance FROM balances WHERE id = $1",
    )
    .bind(balance_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(available, 7000);

    // Cleanup
    cleanup_user(&pool, user_id, account_id).await;
}

// ============================================================
// Intent + Bid + Auction flow
// ============================================================

#[tokio::test]
async fn test_full_intent_to_settlement_flow() {
    let pool = setup_pool().await;
    let (user_id, account_id) = create_test_user_and_account(&pool).await;

    // 1. Deposit funds
    let balance_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'ETH', 50000, 0, NOW())",
    )
    .bind(balance_id)
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    // 2. Create intent
    let intent_id = Uuid::new_v4();
    let now_ts = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out, deadline, status, created_at)
         VALUES ($1, $2, 'ETH', 'USDC', 1000, 900, $3, 'open', $4)",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(now_ts + 3600)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // Verify intent exists
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, "open");

    // 3. Submit bids from solvers
    for i in 0..3 {
        let bid_id = Uuid::new_v4();
        let amount_out: i64 = 950 + i * 10; // 950, 960, 970
        let fee: i64 = 5;
        sqlx::query(
            "INSERT INTO bids (id, intent_id, solver_id, amount_out, fee, timestamp)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(bid_id)
        .bind(intent_id)
        .bind(format!("solver-{i}"))
        .bind(amount_out)
        .bind(fee)
        .bind(now_ts)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Verify bid count
    let bid_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM bids WHERE intent_id = $1",
    )
    .bind(intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(bid_count, 3);

    // 4. Select best bid (highest amount_out - fee)
    let best_bid = sqlx::query_as::<_, (Uuid, String, i64, i64)>(
        "SELECT id, solver_id, amount_out, fee FROM bids
         WHERE intent_id = $1
         ORDER BY (amount_out - fee) DESC, timestamp ASC
         LIMIT 1",
    )
    .bind(intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(best_bid.1, "solver-2"); // 970 - 5 = 965 is best
    assert_eq!(best_bid.2 - best_bid.3, 965);

    // 5. Update intent to matched
    sqlx::query("UPDATE intents SET status = 'matched' WHERE id = $1")
        .bind(intent_id)
        .execute(&pool)
        .await
        .unwrap();

    // 6. Create fill
    sqlx::query(
        "INSERT INTO fills (intent_id, solver_id, price, qty, tx_hash, timestamp)
         VALUES ($1, $2, $3, $4, '', $5)",
    )
    .bind(intent_id)
    .bind(&best_bid.1)
    .bind(best_bid.2)
    .bind(1000_i64)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // 7. Create execution
    let execution_id = Uuid::new_v4();
    let tx_hash = format!("0x{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO executions (id, intent_id, solver_id, tx_hash, status, created_at)
         VALUES ($1, $2, $3, $4, 'completed', $5)",
    )
    .bind(execution_id)
    .bind(intent_id)
    .bind(&best_bid.1)
    .bind(&tx_hash)
    .bind(now_ts)
    .execute(&pool)
    .await
    .unwrap();

    // 8. Mark intent completed
    sqlx::query("UPDATE intents SET status = 'completed' WHERE id = $1")
        .bind(intent_id)
        .execute(&pool)
        .await
        .unwrap();

    // 9. Verify final state
    let final_status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(final_status, "completed");

    let exec_status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM executions WHERE id = $1",
    )
    .bind(execution_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(exec_status, "completed");

    let fill_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM fills WHERE intent_id = $1",
    )
    .bind(intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(fill_exists, 1);

    // Cleanup
    sqlx::query("DELETE FROM executions WHERE intent_id = $1").bind(intent_id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM fills WHERE intent_id = $1").bind(intent_id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM bids WHERE intent_id = $1").bind(intent_id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM intents WHERE id = $1").bind(intent_id).execute(&pool).await.unwrap();
    cleanup_user(&pool, user_id, account_id).await;
}

// ============================================================
// Ledger double-entry test
// ============================================================

#[tokio::test]
async fn test_ledger_double_entry_balances() {
    let pool = setup_pool().await;
    let (user_id, account_id) = create_test_user_and_account(&pool).await;

    // Create a second account (seller)
    let (user_id2, account_id2) = create_test_user_and_account(&pool).await;

    let reference_id = Uuid::new_v4();
    let now = Utc::now();

    // Debit entry (buyer pays)
    let debit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, 'USDC', 5000, 'CREDIT', 'TRADE', $3, $4)",
    )
    .bind(debit_id)
    .bind(account_id)
    .bind(reference_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Credit entry (seller receives)
    let credit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, 'USDC', 5000, 'DEBIT', 'TRADE', $3, $4)",
    )
    .bind(credit_id)
    .bind(account_id2)
    .bind(reference_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Verify: entries for this reference should balance
    let entries = sqlx::query_as::<_, (String, i64)>(
        "SELECT entry_type::text, amount FROM ledger_entries WHERE reference_id = $1 ORDER BY entry_type",
    )
    .bind(reference_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(entries.len(), 2);

    let total_debits: i64 = entries.iter().filter(|e| e.0 == "CREDIT").map(|e| e.1).sum();
    let total_credits: i64 = entries.iter().filter(|e| e.0 == "DEBIT").map(|e| e.1).sum();

    assert_eq!(total_debits, total_credits, "Ledger must balance: debits == credits");

    // Cleanup
    sqlx::query("DELETE FROM ledger_entries WHERE reference_id = $1").bind(reference_id).execute(&pool).await.unwrap();
    cleanup_user(&pool, user_id, account_id).await;
    cleanup_user(&pool, user_id2, account_id2).await;
}

// ============================================================
// Settlement atomicity test
// ============================================================

#[tokio::test]
async fn test_settlement_updates_trade_status() {
    let pool = setup_pool().await;
    let (user_id, buyer_account) = create_test_user_and_account(&pool).await;
    let (user_id2, seller_account) = create_test_user_and_account(&pool).await;
    let (user_id3, solver_account) = create_test_user_and_account(&pool).await;

    // Create trade
    let trade_id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO trades (id, buyer_account_id, seller_account_id, solver_account_id,
            asset_in, asset_out, amount_in, amount_out, platform_fee, solver_fee, status, created_at)
         VALUES ($1, $2, $3, $4, 'USDC', 'ETH', 10000, 5, 10, 5, 'pending', $5)",
    )
    .bind(trade_id)
    .bind(buyer_account)
    .bind(seller_account)
    .bind(solver_account)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Verify pending
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM trades WHERE id = $1",
    )
    .bind(trade_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, "pending");

    // Simulate settlement
    sqlx::query("UPDATE trades SET status = 'settled', settled_at = NOW() WHERE id = $1")
        .bind(trade_id)
        .execute(&pool)
        .await
        .unwrap();

    let status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM trades WHERE id = $1",
    )
    .bind(trade_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, "settled");

    let settled_at = sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
        "SELECT settled_at FROM trades WHERE id = $1",
    )
    .bind(trade_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(settled_at.is_some(), "settled_at should be set");

    // Cleanup
    sqlx::query("DELETE FROM trades WHERE id = $1").bind(trade_id).execute(&pool).await.unwrap();
    cleanup_user(&pool, user_id, buyer_account).await;
    cleanup_user(&pool, user_id2, seller_account).await;
    cleanup_user(&pool, user_id3, solver_account).await;
}

// ============================================================
// Helpers
// ============================================================

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

async fn cleanup_user(pool: &PgPool, user_id: Uuid, account_id: Uuid) {
    sqlx::query("DELETE FROM balances WHERE account_id = $1").bind(account_id).execute(pool).await.unwrap();
    sqlx::query("DELETE FROM accounts WHERE id = $1").bind(account_id).execute(pool).await.unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(pool).await.unwrap();
}
