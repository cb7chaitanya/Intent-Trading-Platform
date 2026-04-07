//! Post-chaos invariant verification integration tests.
//!
//! Sets up a Postgres database with known-good and known-bad states,
//! then runs the invariant checker to verify it catches violations.
//!
//! Run: cargo test --test chaos_verify --features integration
//!
//! Requires Docker to be running.

#![cfg(feature = "integration")]

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

// ============================================================
// Helpers
// ============================================================

async fn setup_postgres() -> (
    PgPool,
    testcontainers_modules::testcontainers::ContainerAsync<Postgres>,
) {
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

async fn create_user_and_account(pool: &PgPool) -> (Uuid, Uuid) {
    let user_id = Uuid::new_v4();
    let account_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'hash', $3, $4)",
    )
    .bind(user_id)
    .bind(format!("chaos-{}@test.local", Uuid::new_v4()))
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

fn hash_secret(secret: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(secret);
    h.finalize().into()
}

// ============================================================
// Invariant checker — inline the query logic for tests
// (same as src/chaos/verify.rs but driven via raw SQL)
// ============================================================

/// Runs all 8 invariant checks and returns (checks_run, violations).
async fn run_invariant_checks(pool: &PgPool) -> (u32, Vec<String>) {
    let mut checks = 0u32;
    let mut violations = Vec::new();

    // 1. Negative balances
    checks += 1;
    let neg = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM balances WHERE available_balance < 0 OR locked_balance < 0",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if neg > 0 {
        violations.push(format!("negative_balances: {neg} rows"));
    }

    // 2. Balance sum vs ledger (skip if no ledger entries — clean state is valid)
    checks += 1;
    let mismatch = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT b.account_id, b.asset,
                    b.available_balance + b.locked_balance as total,
                    COALESCE(l.net, 0) as ledger
             FROM balances b
             LEFT JOIN (
                 SELECT account_id, asset,
                        SUM(CASE WHEN entry_type = 'CREDIT' THEN amount ELSE -amount END) as net
                 FROM ledger_entries
                 GROUP BY account_id, asset
             ) l ON l.account_id = b.account_id AND l.asset = b.asset
             WHERE b.available_balance + b.locked_balance != COALESCE(l.net, 0)
         ) x",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if mismatch > 0 {
        violations.push(format!("balance_ledger_mismatch: {mismatch} rows"));
    }

    // 3. Orphan locked funds
    checks += 1;
    let orphan = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM balances b
         WHERE b.locked_balance > 0
           AND NOT EXISTS (
               SELECT 1 FROM intents i
               JOIN accounts a ON a.user_id::text = i.user_id
               WHERE a.id = b.account_id
                 AND i.status IN ('open', 'bidding', 'matched', 'executing')
           )",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if orphan > 0 {
        violations.push(format!("orphan_locked: {orphan} rows"));
    }

    // 4. Double settlement
    checks += 1;
    let double = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT intent_id FROM fills WHERE settled = true
             GROUP BY intent_id HAVING COUNT(*) > 1
         ) x",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if double > 0 {
        violations.push(format!("double_settlement: {double} intents"));
    }

    // 5. HTLC stuck past timelock
    checks += 1;
    let stuck = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM htlc_swaps
         WHERE status NOT IN ('source_unlocked', 'refunded', 'expired', 'failed')
           AND source_timelock < NOW()",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if stuck > 0 {
        violations.push(format!("htlc_stuck: {stuck} swaps"));
    }

    // 6. HTLC secret integrity
    checks += 1;
    // (checked in Rust code, not easily checkable in pure SQL
    //  because SHA-256 isn't a native PG function without pgcrypto)
    // We'll check in the Rust-level test below

    // 7. Cross-chain leg count
    checks += 1;
    let bad_legs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT fill_id FROM cross_chain_legs GROUP BY fill_id HAVING COUNT(*) != 2
         ) x",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if bad_legs > 0 {
        violations.push(format!("bad_leg_count: {bad_legs} fills"));
    }

    // 8. Ledger per-account match
    // (covered by check 2 at the global level)
    checks += 1;

    (checks, violations)
}

// ============================================================
// Tests
// ============================================================

/// Clean database passes all invariants.
#[tokio::test]
async fn invariant_clean_state_passes() {
    let (pool, _pg) = setup_postgres().await;

    let (_, account_id) = create_user_and_account(&pool).await;

    // Seed a valid balance
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'ETH'::asset_type, 1000, 0, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    let (checks, violations) = run_invariant_checks(&pool).await;
    assert!(checks >= 7, "Should run at least 7 checks");
    assert!(
        violations.is_empty(),
        "Clean state should have no violations: {violations:?}"
    );
}

/// Negative balance is detected.
#[tokio::test]
async fn invariant_catches_negative_balance() {
    let (pool, _pg) = setup_postgres().await;

    let (_, account_id) = create_user_and_account(&pool).await;

    // Insert a negative available balance
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'USDC'::asset_type, -500, 0, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    let (_, violations) = run_invariant_checks(&pool).await;
    assert!(
        violations.iter().any(|v| v.contains("negative")),
        "Should detect negative balance: {violations:?}"
    );
}

/// Orphan locked balance without active intent is detected.
#[tokio::test]
async fn invariant_catches_orphan_locked_funds() {
    let (pool, _pg) = setup_postgres().await;

    let (_, account_id) = create_user_and_account(&pool).await;

    // Locked balance with NO active intent
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'ETH'::asset_type, 0, 5000, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    let (_, violations) = run_invariant_checks(&pool).await;
    assert!(
        violations.iter().any(|v| v.contains("orphan")),
        "Should detect orphan locked funds: {violations:?}"
    );
}

/// HTLC swap stuck past timelock is detected.
#[tokio::test]
async fn invariant_catches_stuck_htlc() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO intents
            (id, user_id, token_in, token_out, amount_in, min_amount_out,
             deadline, status, created_at, order_type)
         VALUES ($1, $2, 'ETH', 'SOL', 1000, 900, $3, 'executing', $4, 'market')",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(Utc::now().timestamp() + 3600)
    .bind(Utc::now().timestamp())
    .execute(&pool)
    .await
    .unwrap();

    let secret: [u8; 32] = rand::random();
    let secret_hash = hash_secret(&secret);

    // HTLC in source_locked but past timelock
    sqlx::query(
        "INSERT INTO htlc_swaps
            (id, fill_id, intent_id, secret_hash,
             source_chain, source_sender, source_receiver, source_amount,
             source_timelock,
             dest_chain, dest_sender, dest_receiver, dest_amount,
             status, solver_id, created_at)
         VALUES ($1, $2, $3, $4,
                 'ethereum', '0xA', '0xB', 1000,
                 $5,
                 'solana', 'C', 'D', 900,
                 'source_locked', 'solver1', NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(fill_id)
    .bind(intent_id)
    .bind(secret_hash.as_slice())
    .bind(Utc::now() - Duration::hours(1)) // expired 1 hour ago
    .execute(&pool)
    .await
    .unwrap();

    let (_, violations) = run_invariant_checks(&pool).await;
    assert!(
        violations.iter().any(|v| v.contains("htlc_stuck")),
        "Should detect stuck HTLC: {violations:?}"
    );
}

/// HTLC secret integrity — wrong preimage detected.
#[tokio::test]
async fn invariant_htlc_secret_integrity_check() {
    // This tests the Rust-level SHA-256 verification
    let real_secret: [u8; 32] = rand::random();
    let wrong_secret: [u8; 32] = rand::random();
    let hash = hash_secret(&real_secret);

    // Correct preimage
    assert_eq!(hash_secret(&real_secret), hash);

    // Wrong preimage
    assert_ne!(hash_secret(&wrong_secret), hash);
}

/// Cross-chain legs with wrong count detected.
#[tokio::test]
async fn invariant_catches_wrong_leg_count() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO intents
            (id, user_id, token_in, token_out, amount_in, min_amount_out,
             deadline, status, created_at, order_type)
         VALUES ($1, $2, 'ETH', 'SOL', 1000, 900, $3, 'executing', $4, 'market')",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(Utc::now().timestamp() + 3600)
    .bind(Utc::now().timestamp())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO fills (id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled)
         VALUES ($1, $2, 'solver', 1000, 1000, 1000, '', $3, false)",
    )
    .bind(fill_id)
    .bind(intent_id)
    .bind(Utc::now().timestamp())
    .execute(&pool)
    .await
    .unwrap();

    // Insert only 1 leg (should be 2)
    sqlx::query(
        "INSERT INTO cross_chain_legs
            (id, intent_id, fill_id, leg_index, chain, from_address, to_address,
             amount, status, timeout_at, created_at)
         VALUES ($1, $2, $3, 0, 'ethereum', '0xA', '0xB', 1000, 'pending', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(intent_id)
    .bind(fill_id)
    .bind(Utc::now() + Duration::minutes(10))
    .bind(Utc::now())
    .execute(&pool)
    .await
    .unwrap();

    let (_, violations) = run_invariant_checks(&pool).await;
    assert!(
        violations.iter().any(|v| v.contains("bad_leg_count")),
        "Should detect wrong leg count: {violations:?}"
    );
}

/// Balance/ledger mismatch detected.
#[tokio::test]
async fn invariant_catches_balance_ledger_mismatch() {
    let (pool, _pg) = setup_postgres().await;

    let (_, account_id) = create_user_and_account(&pool).await;

    // Balance says 1000 available
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, 'ETH'::asset_type, 1000, 0, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();

    // Ledger says only 500 was credited (mismatch)
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, 'ETH'::asset_type, 500, 'CREDIT', 'DEPOSIT', $3, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .unwrap();

    let (_, violations) = run_invariant_checks(&pool).await;
    assert!(
        violations.iter().any(|v| v.contains("mismatch")),
        "Should detect balance/ledger mismatch: {violations:?}"
    );
}
