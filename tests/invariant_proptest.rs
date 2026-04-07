//! Property-based invariant tests.
//!
//! Generates random sequences of financial operations and verifies that
//! system invariants hold after every step. Uses testcontainers for
//! isolated Postgres instances.
//!
//! Run: cargo test --test invariant_proptest --features integration -- --nocapture
//!
//! Requires Docker.

#![cfg(feature = "integration")]

use chrono::Utc;
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

// ============================================================
// Setup
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
        .max_connections(10)
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
    .bind(format!("prop-{}@test.local", Uuid::new_v4()))
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

// ============================================================
// Invariant checkers (run against raw DB state)
// ============================================================

/// INV-1: For each asset, SUM(available+locked) == SUM(credits) - SUM(debits)
async fn check_balance_conservation(pool: &PgPool) -> Result<(), String> {
    let mismatches = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT b.asset::text,
                COALESCE(b.total, 0) as balance_total,
                COALESCE(l.net, 0) as ledger_net
         FROM (SELECT asset, SUM(available_balance + locked_balance) as total FROM balances GROUP BY asset) b
         FULL JOIN (SELECT asset, SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net FROM ledger_entries GROUP BY asset) l
         ON b.asset = l.asset
         WHERE COALESCE(b.total, 0) != COALESCE(l.net, 0)",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("query: {e}"))?;

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Balance conservation violated: {:?}",
            mismatches
                .iter()
                .map(|(a, b, l)| format!("{a}: balance={b} ledger={l}"))
                .collect::<Vec<_>>()
        ))
    }
}

/// INV-2: No fill settled more than once per intent
async fn check_no_double_settlement(pool: &PgPool) -> Result<(), String> {
    let doubles = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT intent_id FROM fills WHERE settled = TRUE
             GROUP BY intent_id HAVING COUNT(*) > 1
         ) x",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("query: {e}"))?;

    if doubles == 0 {
        Ok(())
    } else {
        Err(format!("{doubles} intents have multiple settled fills"))
    }
}

/// INV-6: No negative balances
async fn check_no_negative_balances(pool: &PgPool) -> Result<(), String> {
    let negatives = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM balances
         WHERE available_balance < 0 OR locked_balance < 0",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("query: {e}"))?;

    if negatives == 0 {
        Ok(())
    } else {
        Err(format!("{negatives} balances are negative"))
    }
}

/// Run all invariants. Returns list of violations.
async fn check_all_invariants(pool: &PgPool) -> Vec<String> {
    let mut violations = Vec::new();

    if let Err(e) = check_balance_conservation(pool).await {
        violations.push(format!("INV-1: {e}"));
    }
    if let Err(e) = check_no_double_settlement(pool).await {
        violations.push(format!("INV-2: {e}"));
    }
    if let Err(e) = check_no_negative_balances(pool).await {
        violations.push(format!("INV-6: {e}"));
    }

    violations
}

// ============================================================
// Operations (atomic DB mutations that mirror service calls)
// ============================================================

/// Deposit: credit available_balance + insert ledger entry.
async fn op_deposit(pool: &PgPool, account_id: Uuid, asset: &str, amount: i64) {
    let mut tx = pool.begin().await.unwrap();

    // Upsert balance
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, $3::asset_type, $4, 0, NOW())
         ON CONFLICT (account_id, asset) DO UPDATE SET
             available_balance = balances.available_balance + $4,
             updated_at = NOW()",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(asset)
    .bind(amount)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Ledger entry
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, $3::asset_type, $4, 'CREDIT', 'DEPOSIT', $5, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(asset)
    .bind(amount)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
}

/// Withdraw: debit available_balance + insert ledger entry.
/// Returns false if insufficient balance.
async fn op_withdraw(pool: &PgPool, account_id: Uuid, asset: &str, amount: i64) -> bool {
    let mut tx = pool.begin().await.unwrap();

    let available: i64 = sqlx::query_scalar(
        "SELECT COALESCE(available_balance, 0) FROM balances WHERE account_id = $1 AND asset = $2::asset_type",
    )
    .bind(account_id)
    .bind(asset)
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .unwrap_or(0);

    if available < amount {
        return false;
    }

    sqlx::query(
        "UPDATE balances SET available_balance = available_balance - $1, updated_at = NOW()
         WHERE account_id = $2 AND asset = $3::asset_type",
    )
    .bind(amount)
    .bind(account_id)
    .bind(asset)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, $3::asset_type, $4, 'DEBIT', 'WITHDRAWAL', $5, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(asset)
    .bind(amount)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    true
}

/// Transfer: atomic debit from + credit to + double-entry ledger.
async fn op_transfer(
    pool: &PgPool,
    from_account: Uuid,
    to_account: Uuid,
    asset: &str,
    amount: i64,
) -> bool {
    let mut tx = pool.begin().await.unwrap();

    let available: i64 = sqlx::query_scalar(
        "SELECT COALESCE(available_balance, 0) FROM balances WHERE account_id = $1 AND asset = $2::asset_type FOR UPDATE",
    )
    .bind(from_account)
    .bind(asset)
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .unwrap_or(0);

    if available < amount {
        return false;
    }

    sqlx::query(
        "UPDATE balances SET available_balance = available_balance - $1, updated_at = NOW()
         WHERE account_id = $2 AND asset = $3::asset_type",
    )
    .bind(amount)
    .bind(from_account)
    .bind(asset)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Ensure recipient balance row exists
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, $3::asset_type, 0, 0, NOW())
         ON CONFLICT (account_id, asset) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(to_account)
    .bind(asset)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE balances SET available_balance = available_balance + $1, updated_at = NOW()
         WHERE account_id = $2 AND asset = $3::asset_type",
    )
    .bind(amount)
    .bind(to_account)
    .bind(asset)
    .execute(&mut *tx)
    .await
    .unwrap();

    let ref_id = Uuid::new_v4();
    // Debit from
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, $3::asset_type, $4, 'DEBIT', 'TRADE', $5, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(from_account)
    .bind(asset)
    .bind(amount)
    .bind(ref_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Credit to
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, $3::asset_type, $4, 'CREDIT', 'TRADE', $5, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(to_account)
    .bind(asset)
    .bind(amount)
    .bind(ref_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    true
}

// ============================================================
// Property-based tests
// ============================================================

/// P1: Random deposit/withdraw sequences preserve balance conservation.
#[tokio::test]
async fn prop_random_deposits_withdrawals_conserve_balance() {
    let (pool, _pg) = setup_postgres().await;
    let (_, account_a) = create_user_and_account(&pool).await;
    let (_, account_b) = create_user_and_account(&pool).await;

    // Run 200 random operations
    for i in 0..200u32 {
        let amount = (rand::random::<u64>() % 10_000 + 1) as i64;
        let account = if rand::random::<bool>() {
            account_a
        } else {
            account_b
        };
        let asset = match rand::random::<u8>() % 4 {
            0 => "USDC",
            1 => "ETH",
            2 => "BTC",
            _ => "SOL",
        };

        match rand::random::<u8>() % 3 {
            0 => {
                // Deposit
                op_deposit(&pool, account, asset, amount).await;
            }
            1 => {
                // Withdraw (may fail if insufficient)
                let _ = op_withdraw(&pool, account, asset, amount).await;
            }
            _ => {
                // Transfer between accounts (may fail if insufficient)
                let (from, to) = if rand::random::<bool>() {
                    (account_a, account_b)
                } else {
                    (account_b, account_a)
                };
                let _ = op_transfer(&pool, from, to, asset, amount).await;
            }
        }

        // Check invariants every 20 operations
        if (i + 1) % 20 == 0 {
            let violations = check_all_invariants(&pool).await;
            assert!(
                violations.is_empty(),
                "Invariant violation after operation {}: {:?}",
                i + 1,
                violations
            );
        }
    }

    // Final check
    let violations = check_all_invariants(&pool).await;
    assert!(
        violations.is_empty(),
        "Final invariant violations: {:?}",
        violations
    );
}

/// P4: Concurrent settlement attempts on same fill produce exactly one success.
#[tokio::test]
async fn prop_concurrent_settlement_no_double() {
    let (pool, _pg) = setup_postgres().await;
    let (user_id, account_id) = create_user_and_account(&pool).await;

    // Seed balance
    op_deposit(&pool, account_id, "ETH", 100_000).await;

    // Create intent + fill
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out, deadline, status, created_at, order_type)
         VALUES ($1, $2, 'ETH', 'USDC', 1000, 900, $3, 'executing', $4, 'market')",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(now.timestamp() + 3600)
    .bind(now.timestamp())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO fills (id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled)
         VALUES ($1, $2, 'solver-1', 1000, 1000, 1000, '', $3, false)",
    )
    .bind(fill_id)
    .bind(intent_id)
    .bind(now.timestamp())
    .execute(&pool)
    .await
    .unwrap();

    // Spawn 20 concurrent "settle" attempts
    let mut handles = Vec::new();
    for _ in 0..20 {
        let pool = pool.clone();
        let fid = fill_id;
        handles.push(tokio::spawn(async move {
            // Simulate settle_fill: SELECT FOR UPDATE + check + mark settled
            let mut tx = pool.begin().await.unwrap();

            let already: bool = sqlx::query_scalar(
                "SELECT settled FROM fills WHERE id = $1 FOR UPDATE",
            )
            .bind(fid)
            .fetch_one(&mut *tx)
            .await
            .unwrap();

            if already {
                return false; // already settled
            }

            sqlx::query("UPDATE fills SET settled = TRUE, settled_at = NOW() WHERE id = $1")
                .bind(fid)
                .execute(&mut *tx)
                .await
                .unwrap();

            tx.commit().await.unwrap();
            true // we were the one that settled it
        }));
    }

    let mut successes = 0;
    for h in handles {
        if h.await.unwrap() {
            successes += 1;
        }
    }

    assert_eq!(successes, 1, "Exactly one concurrent settler should succeed");

    // INV-2: no double settlement
    let violations = check_no_double_settlement(&pool).await;
    assert!(violations.is_ok(), "Double settlement detected: {violations:?}");
}

/// P3 (simplified): HTLC claim vs refund mutual exclusion via status guard.
#[tokio::test]
async fn prop_htlc_claim_refund_mutual_exclusion() {
    let (pool, _pg) = setup_postgres().await;
    let (user_id, _) = create_user_and_account(&pool).await;

    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let swap_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out, deadline, status, created_at, order_type)
         VALUES ($1, $2, 'ETH', 'SOL', 1000, 900, $3, 'executing', $4, 'market')",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(now.timestamp() + 3600)
    .bind(now.timestamp())
    .execute(&pool)
    .await
    .unwrap();

    let secret: [u8; 32] = rand::random();
    let hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&secret);
        let result: [u8; 32] = h.finalize().into();
        result
    };

    // Insert HTLC in source_locked state with expired timelock
    sqlx::query(
        "INSERT INTO htlc_swaps (id, fill_id, intent_id, secret_hash, secret,
             source_chain, source_sender, source_receiver, source_amount, source_timelock,
             dest_chain, dest_sender, dest_receiver, dest_amount,
             status, solver_id, created_at, locked_at, dest_lock_tx)
         VALUES ($1, $2, $3, $4, $5,
                 'ethereum', '0xA', '0xB', 1000, $6,
                 'solana', 'C', 'D', 900,
                 'source_locked', 'solver1', NOW(), NOW(), '0xdestlock')",
    )
    .bind(swap_id)
    .bind(fill_id)
    .bind(intent_id)
    .bind(hash.as_slice())
    .bind(secret.as_slice())
    .bind(now - chrono::Duration::seconds(1)) // just expired
    .execute(&pool)
    .await
    .unwrap();

    // Race: one task tries to claim, another tries to refund
    let pool_claim = pool.clone();
    let pool_refund = pool.clone();

    let claim_handle = tokio::spawn(async move {
        sqlx::query(
            "UPDATE htlc_swaps SET status = 'dest_claimed', claimed_at = NOW()
             WHERE id = $1 AND status = 'source_locked'",
        )
        .bind(swap_id)
        .execute(&pool_claim)
        .await
        .unwrap()
        .rows_affected()
    });

    let refund_handle = tokio::spawn(async move {
        sqlx::query(
            "UPDATE htlc_swaps SET status = 'refunded', completed_at = NOW()
             WHERE id = $1 AND status IN ('created', 'source_locked')",
        )
        .bind(swap_id)
        .execute(&pool_refund)
        .await
        .unwrap()
        .rows_affected()
    });

    let claim_affected = claim_handle.await.unwrap();
    let refund_affected = refund_handle.await.unwrap();

    // Exactly one should succeed (both WHERE clauses require source_locked,
    // but only one UPDATE can change it first)
    assert_eq!(
        claim_affected + refund_affected,
        1,
        "Exactly one of claim/refund should succeed: claim={claim_affected} refund={refund_affected}"
    );

    // Verify final status is one or the other, not both
    let final_status: String = sqlx::query_scalar(
        "SELECT status::text FROM htlc_swaps WHERE id = $1",
    )
    .bind(swap_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        final_status == "dest_claimed" || final_status == "refunded",
        "Final status must be one of claim/refund, got: {final_status}"
    );
}

/// Verify that zero-amount operations don't violate conservation.
#[tokio::test]
async fn prop_zero_and_edge_amounts() {
    let (pool, _pg) = setup_postgres().await;
    let (_, account) = create_user_and_account(&pool).await;

    // Deposit 0 — should still create ledger entry
    op_deposit(&pool, account, "USDC", 0).await;

    // Deposit max reasonable amount
    op_deposit(&pool, account, "USDC", 1_000_000_000).await;

    // Withdraw 0
    let ok = op_withdraw(&pool, account, "USDC", 0).await;
    assert!(ok);

    // Withdraw exact balance
    let ok = op_withdraw(&pool, account, "USDC", 1_000_000_000).await;
    assert!(ok);

    // Withdraw more than balance
    let ok = op_withdraw(&pool, account, "USDC", 1).await;
    assert!(!ok, "Should fail: insufficient balance");

    let violations = check_all_invariants(&pool).await;
    assert!(
        violations.is_empty(),
        "Edge case violations: {:?}",
        violations
    );
}
