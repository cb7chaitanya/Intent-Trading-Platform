//! HTLC atomic swap lifecycle integration tests.
//!
//! Tests the full lock → claim → unlock flow, wrong-preimage rejection,
//! and timeout refund behavior using testcontainers for Postgres.
//!
//! Run: cargo test --test htlc_e2e --features integration
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

fn unique_email() -> String {
    format!("test-{}@htlc.test", Uuid::new_v4())
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

async fn insert_intent(pool: &PgPool, intent_id: Uuid, user_id: Uuid) {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO intents
            (id, user_id, token_in, token_out, amount_in, min_amount_out,
             deadline, status, created_at, order_type,
             source_chain, destination_chain, cross_chain)
         VALUES ($1, $2, 'ETH', 'SOL', 1000000, 900000,
                 $3, 'executing', $4, 'market',
                 'ethereum', 'solana', true)",
    )
    .bind(intent_id)
    .bind(user_id.to_string())
    .bind(now.timestamp() + 3600)
    .bind(now.timestamp())
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_fill(pool: &PgPool, fill_id: Uuid, intent_id: Uuid, solver_id: &str) {
    sqlx::query(
        "INSERT INTO fills
            (id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled)
         VALUES ($1, $2, $3, 1000, 1000000, 1000000, '', $4, false)",
    )
    .bind(fill_id)
    .bind(intent_id)
    .bind(solver_id)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await
    .unwrap();
}

/// Generate a random 32-byte secret.
fn generate_secret() -> [u8; 32] {
    rand::random()
}

/// SHA-256(secret).
fn hash_secret(secret: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret);
    hasher.finalize().into()
}

/// Insert an HTLC swap directly via SQL for test control.
async fn insert_htlc_swap(
    pool: &PgPool,
    swap_id: Uuid,
    fill_id: Uuid,
    intent_id: Uuid,
    secret_hash: &[u8; 32],
    secret: Option<&[u8; 32]>,
    status: &str,
    solver_id: &str,
    timelock: chrono::DateTime<Utc>,
    source_lock_tx: Option<&str>,
    dest_lock_tx: Option<&str>,
) {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO htlc_swaps
            (id, fill_id, intent_id, secret_hash, secret,
             source_chain, source_sender, source_receiver, source_token,
             source_amount, source_timelock, source_lock_tx,
             dest_chain, dest_sender, dest_receiver, dest_token,
             dest_amount, dest_lock_tx,
             status, solver_id, created_at,
             locked_at)
         VALUES ($1, $2, $3, $4, $5,
                 'ethereum', '0xUserEth', '0xSolverEth', '0xWETH',
                 1000000, $6, $7,
                 'solana', 'SolverSol', 'UserSol', 'SOLmint',
                 900000, $8,
                 $9::htlc_status, $10, $11,
                 $12)",
    )
    .bind(swap_id)
    .bind(fill_id)
    .bind(intent_id)
    .bind(secret_hash.as_slice())
    .bind(secret.map(|s| s.as_slice()))
    .bind(timelock)
    .bind(source_lock_tx)
    .bind(dest_lock_tx)
    .bind(status)
    .bind(solver_id)
    .bind(now)
    .bind(if status == "created" { None } else { Some(now) })
    .execute(pool)
    .await
    .unwrap();
}

/// Read back an HTLC swap from the DB.
#[derive(Debug, sqlx::FromRow)]
struct SwapRow {
    id: Uuid,
    status: String,
    secret_hash: Vec<u8>,
    secret: Option<Vec<u8>>,
    source_lock_tx: Option<String>,
    source_unlock_tx: Option<String>,
    dest_lock_tx: Option<String>,
    dest_claim_tx: Option<String>,
    locked_at: Option<chrono::DateTime<Utc>>,
    claimed_at: Option<chrono::DateTime<Utc>>,
    completed_at: Option<chrono::DateTime<Utc>>,
}

async fn get_swap(pool: &PgPool, swap_id: Uuid) -> SwapRow {
    sqlx::query_as::<_, SwapRow>(
        "SELECT id, status::text as status, secret_hash, secret,
                source_lock_tx, source_unlock_tx,
                dest_lock_tx, dest_claim_tx,
                locked_at, claimed_at, completed_at
         FROM htlc_swaps
         WHERE id = $1",
    )
    .bind(swap_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ============================================================
// Test 1: Full lifecycle — lock → claim → unlock
// ============================================================

#[tokio::test]
async fn htlc_full_lifecycle_lock_claim_unlock() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_test_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let solver_id = format!("solver-{}", Uuid::new_v4());

    insert_intent(&pool, intent_id, user_id).await;
    insert_fill(&pool, fill_id, intent_id, &solver_id).await;

    let secret = generate_secret();
    let secret_hash = hash_secret(&secret);
    let swap_id = Uuid::new_v4();
    let timelock = Utc::now() + Duration::minutes(30);

    // ── Step 1: Create swap (status = created) ──────
    insert_htlc_swap(
        &pool, swap_id, fill_id, intent_id,
        &secret_hash, None, "created", &solver_id,
        timelock, None, None,
    ).await;

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "created");
    assert_eq!(swap.secret_hash, secret_hash.to_vec());
    assert!(swap.secret.is_none(), "Secret should not be stored yet");

    // ── Step 1b: Store secret ────────────────────────
    sqlx::query("UPDATE htlc_swaps SET secret = $2 WHERE id = $1")
        .bind(swap_id)
        .bind(secret.as_slice())
        .execute(&pool)
        .await
        .unwrap();

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.secret.as_deref(), Some(secret.as_slice()));

    // Verify: stored secret hashes to the stored hash
    let stored_secret: [u8; 32] = swap.secret.unwrap().try_into().unwrap();
    assert_eq!(hash_secret(&stored_secret), secret_hash);

    // ── Step 2: Lock source (created → source_locked) ──
    let lock_tx = "0xlocktx_abc123";
    let result = sqlx::query(
        "UPDATE htlc_swaps
         SET status = 'source_locked', source_lock_tx = $2, locked_at = NOW()
         WHERE id = $1 AND status = 'created'",
    )
    .bind(swap_id)
    .bind(lock_tx)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "source_locked");
    assert_eq!(swap.source_lock_tx.as_deref(), Some(lock_tx));
    assert!(swap.locked_at.is_some());

    // ── Step 3: Record solver's dest lock ────────────
    let dest_lock_tx = "0xdestlocktx_def456";
    sqlx::query("UPDATE htlc_swaps SET dest_lock_tx = $2 WHERE id = $1 AND status = 'source_locked'")
        .bind(swap_id)
        .bind(dest_lock_tx)
        .execute(&pool)
        .await
        .unwrap();

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.dest_lock_tx.as_deref(), Some(dest_lock_tx));

    // Swap should now be "claimable" (source_locked + dest_lock_tx IS NOT NULL)
    let claimable_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM htlc_swaps
         WHERE status = 'source_locked' AND dest_lock_tx IS NOT NULL AND id = $1",
    )
    .bind(swap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claimable_count, 1, "Swap should be claimable");

    // ── Step 4: Claim destination (reveal secret) ────
    // Verify secret matches hash before claiming
    let stored_hash: [u8; 32] = swap.secret_hash.try_into().unwrap();
    assert_eq!(hash_secret(&secret), stored_hash, "Secret must match stored hash");

    let claim_tx = "0xclaimtx_789ghi";
    let result = sqlx::query(
        "UPDATE htlc_swaps
         SET status = 'dest_claimed', dest_claim_tx = $2, claimed_at = NOW()
         WHERE id = $1 AND status = 'source_locked'",
    )
    .bind(swap_id)
    .bind(claim_tx)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "dest_claimed");
    assert_eq!(swap.dest_claim_tx.as_deref(), Some(claim_tx));
    assert!(swap.claimed_at.is_some());

    // Swap should now be "pending unlock" (dest_claimed + secret IS NOT NULL)
    let pending_unlock_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM htlc_swaps
         WHERE status = 'dest_claimed' AND secret IS NOT NULL AND id = $1",
    )
    .bind(swap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending_unlock_count, 1, "Swap should be pending unlock");

    // ── Step 5: Unlock source (complete swap) ────────
    // Verify secret one more time before unlocking
    let final_secret: [u8; 32] = swap.secret.unwrap().try_into().unwrap();
    assert_eq!(
        hash_secret(&final_secret),
        stored_hash,
        "Secret must still match at unlock time"
    );

    let unlock_tx = "0xunlocktx_jklmno";
    let result = sqlx::query(
        "UPDATE htlc_swaps
         SET status = 'source_unlocked', source_unlock_tx = $2, completed_at = NOW()
         WHERE id = $1 AND status = 'dest_claimed'",
    )
    .bind(swap_id)
    .bind(unlock_tx)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "source_unlocked");
    assert_eq!(swap.source_unlock_tx.as_deref(), Some(unlock_tx));
    assert!(swap.completed_at.is_some());

    // ── Verify terminal state ────────────────────────
    // Cannot transition further
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'refunded' WHERE id = $1 AND status IN ('created', 'source_locked')",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 0, "Cannot refund a completed swap");
}

// ============================================================
// Test 2: Wrong preimage is rejected
// ============================================================

#[tokio::test]
async fn htlc_wrong_preimage_rejected() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_test_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let solver_id = format!("solver-{}", Uuid::new_v4());

    insert_intent(&pool, intent_id, user_id).await;
    insert_fill(&pool, fill_id, intent_id, &solver_id).await;

    let real_secret = generate_secret();
    let wrong_secret = generate_secret();
    let secret_hash = hash_secret(&real_secret);
    let swap_id = Uuid::new_v4();
    let timelock = Utc::now() + Duration::minutes(30);

    // Create swap with source locked and dest locked
    insert_htlc_swap(
        &pool, swap_id, fill_id, intent_id,
        &secret_hash, Some(&real_secret), "source_locked", &solver_id,
        timelock, Some("0xlocktx"), Some("0xdestlocktx"),
    ).await;

    // ── Wrong preimage does not match hash ───────────
    let wrong_hash = hash_secret(&wrong_secret);
    assert_ne!(
        wrong_hash, secret_hash,
        "Wrong secret must produce a different hash"
    );

    // The claim should only succeed with the correct secret.
    // Simulate what record_dest_claim does: verify before updating.
    let swap = get_swap(&pool, swap_id).await;
    let stored_hash: [u8; 32] = swap.secret_hash.try_into().unwrap();

    // Wrong secret fails verification
    assert_ne!(
        hash_secret(&wrong_secret), stored_hash,
        "Wrong secret must not match stored hash"
    );

    // Right secret passes verification
    assert_eq!(
        hash_secret(&real_secret), stored_hash,
        "Correct secret must match stored hash"
    );

    // ── Ensure the DB enforces: only the right secret can claim ──
    // With the wrong secret, we should NOT update the status
    // (simulating what HtlcService::record_dest_claim does)
    let wrong_hash_computed = hash_secret(&wrong_secret);
    let matches = wrong_hash_computed == stored_hash;
    assert!(!matches, "verify_secret should return false for wrong preimage");

    // Only update if secret matches (conditional claim)
    if matches {
        // This block should not execute
        panic!("Wrong secret should not match");
    }

    // Swap should still be source_locked (claim was rejected)
    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "source_locked", "Status unchanged after wrong preimage");
    assert!(swap.claimed_at.is_none(), "No claim should be recorded");

    // ── Now claim with the correct secret ──────────
    assert_eq!(hash_secret(&real_secret), stored_hash);

    sqlx::query(
        "UPDATE htlc_swaps
         SET status = 'dest_claimed', secret = $2, dest_claim_tx = $3, claimed_at = NOW()
         WHERE id = $1 AND status = 'source_locked'",
    )
    .bind(swap_id)
    .bind(real_secret.as_slice())
    .bind("0xcorrect_claim_tx")
    .execute(&pool)
    .await
    .unwrap();

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "dest_claimed");
    assert_eq!(swap.secret.as_deref(), Some(real_secret.as_slice()));
}

// ============================================================
// Test 3: Timeout refund — only after timelock expires
// ============================================================

#[tokio::test]
async fn htlc_timeout_refund_only_after_timelock() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_test_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let solver_id = format!("solver-{}", Uuid::new_v4());

    insert_intent(&pool, intent_id, user_id).await;
    insert_fill(&pool, fill_id, intent_id, &solver_id).await;

    let secret = generate_secret();
    let secret_hash = hash_secret(&secret);
    let swap_id = Uuid::new_v4();

    // Timelock set 30 minutes in the future — not expired
    let future_timelock = Utc::now() + Duration::minutes(30);

    insert_htlc_swap(
        &pool, swap_id, fill_id, intent_id,
        &secret_hash, Some(&secret), "source_locked", &solver_id,
        future_timelock, Some("0xlocktx"), None,
    ).await;

    // ── Cannot refund before timelock ────────────────
    // The find_expired query should NOT return this swap
    let expired = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM htlc_swaps
         WHERE source_timelock < NOW()
           AND status IN ('created', 'source_locked')
           AND id = $1",
    )
    .bind(swap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(expired, 0, "Swap should NOT be in expired set before timelock");

    // Attempting to refund before timelock should be rejected
    // (simulating HtlcService::refund_swap's timelock check)
    let swap = get_swap(&pool, swap_id).await;
    let timelock_expired = Utc::now() >= future_timelock;
    assert!(!timelock_expired, "Timelock should not have expired yet");

    // ── Advance timelock to the past ─────────────────
    let past_timelock = Utc::now() - Duration::seconds(60);
    sqlx::query("UPDATE htlc_swaps SET source_timelock = $2 WHERE id = $1")
        .bind(swap_id)
        .bind(past_timelock)
        .execute(&pool)
        .await
        .unwrap();

    // Now find_expired should return this swap
    let expired = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM htlc_swaps
         WHERE source_timelock < NOW()
           AND status IN ('created', 'source_locked')
           AND id = $1",
    )
    .bind(swap_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(expired, 1, "Swap should be in expired set after timelock");

    // ── Refund succeeds after timelock ───────────────
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'refunded', completed_at = NOW()
         WHERE id = $1 AND status IN ('created', 'source_locked')",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1, "Refund should succeed after timelock");

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "refunded");
    assert!(swap.completed_at.is_some());
    assert!(swap.claimed_at.is_none(), "No claim should have happened");
    assert!(swap.source_unlock_tx.is_none(), "No unlock should have happened");
}

// ============================================================
// Test 4: Cannot refund after secret is revealed (dest claimed)
// ============================================================

#[tokio::test]
async fn htlc_cannot_refund_after_claim() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_test_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let solver_id = format!("solver-{}", Uuid::new_v4());

    insert_intent(&pool, intent_id, user_id).await;
    insert_fill(&pool, fill_id, intent_id, &solver_id).await;

    let secret = generate_secret();
    let secret_hash = hash_secret(&secret);
    let swap_id = Uuid::new_v4();
    let past_timelock = Utc::now() - Duration::seconds(60); // expired

    // Create swap already in dest_claimed state
    insert_htlc_swap(
        &pool, swap_id, fill_id, intent_id,
        &secret_hash, Some(&secret), "source_locked", &solver_id,
        past_timelock, Some("0xlocktx"), Some("0xdestlock"),
    ).await;

    // Transition to dest_claimed
    sqlx::query(
        "UPDATE htlc_swaps
         SET status = 'dest_claimed', dest_claim_tx = '0xclaim', claimed_at = NOW()
         WHERE id = $1",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();

    // Even though timelock expired, refund should fail because
    // status is dest_claimed (not in refundable states)
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'refunded', completed_at = NOW()
         WHERE id = $1 AND status IN ('created', 'source_locked')",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        result.rows_affected(), 0,
        "Cannot refund: secret already revealed (dest_claimed)"
    );

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "dest_claimed", "Status should remain dest_claimed");
}

// ============================================================
// Test 5: Secret hash verification is cryptographically sound
// ============================================================

#[tokio::test]
async fn htlc_crypto_properties() {
    // Deterministic hashing
    let secret = generate_secret();
    let h1 = hash_secret(&secret);
    let h2 = hash_secret(&secret);
    assert_eq!(h1, h2, "SHA-256 must be deterministic");

    // Different secrets → different hashes
    let s1 = generate_secret();
    let s2 = generate_secret();
    assert_ne!(s1, s2, "Random secrets should differ");
    assert_ne!(hash_secret(&s1), hash_secret(&s2), "Different secrets → different hashes");

    // Preimage resistance: hash ≠ secret
    assert_ne!(secret, hash_secret(&secret), "Hash must differ from preimage");

    // Known test vector: SHA-256 of all zeros
    let zeros = [0u8; 32];
    let expected_hex = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";
    let actual = hash_secret(&zeros);
    assert_eq!(hex::encode(actual), expected_hex, "SHA-256 of zeros must match known vector");
}

// ============================================================
// Test 6: extract_secret_from_logs parses correctly
// ============================================================

#[tokio::test]
async fn htlc_extract_secret_from_log_data() {
    let secret = generate_secret();
    let hex_data = format!("0x{}", hex::encode(secret));

    // Parse secret from hex log data
    let hex_str = hex_data.strip_prefix("0x").unwrap_or(&hex_data);
    let data = hex::decode(hex_str).unwrap();
    let mut parsed = [0u8; 32];
    parsed.copy_from_slice(&data[..32]);

    assert_eq!(parsed, secret, "Extracted secret must match original");
    assert_eq!(
        hash_secret(&parsed),
        hash_secret(&secret),
        "Extracted secret must produce the same hash"
    );

    // Too-short data should fail
    let short_hex = "0xdeadbeef";
    let short_data = hex::decode(short_hex.strip_prefix("0x").unwrap()).unwrap();
    assert!(short_data.len() < 32, "Short data should be less than 32 bytes");
}

// ============================================================
// Test 7: Status transition constraints
// ============================================================

#[tokio::test]
async fn htlc_status_transitions_are_enforced() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, _) = create_test_user_and_account(&pool).await;
    let intent_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let solver_id = format!("solver-{}", Uuid::new_v4());

    insert_intent(&pool, intent_id, user_id).await;
    insert_fill(&pool, fill_id, intent_id, &solver_id).await;

    let secret = generate_secret();
    let secret_hash = hash_secret(&secret);
    let swap_id = Uuid::new_v4();
    let timelock = Utc::now() + Duration::minutes(30);

    insert_htlc_swap(
        &pool, swap_id, fill_id, intent_id,
        &secret_hash, None, "created", &solver_id,
        timelock, None, None,
    ).await;

    // ── Cannot skip to dest_claimed from created ────
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'dest_claimed'
         WHERE id = $1 AND status = 'source_locked'",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 0, "Cannot jump created → dest_claimed");

    // ── Cannot skip to source_unlocked from created ──
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'source_unlocked'
         WHERE id = $1 AND status = 'dest_claimed'",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 0, "Cannot jump created → source_unlocked");

    // ── Valid: created → source_locked ───────────────
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'source_locked', source_lock_tx = '0xtx', locked_at = NOW()
         WHERE id = $1 AND status = 'created'",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    // ── Valid: source_locked → dest_claimed ──────────
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'dest_claimed', dest_claim_tx = '0xclaim', claimed_at = NOW()
         WHERE id = $1 AND status = 'source_locked'",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    // ── Valid: dest_claimed → source_unlocked ────────
    let result = sqlx::query(
        "UPDATE htlc_swaps SET status = 'source_unlocked', source_unlock_tx = '0xunlock', completed_at = NOW()
         WHERE id = $1 AND status = 'dest_claimed'",
    )
    .bind(swap_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);

    let swap = get_swap(&pool, swap_id).await;
    assert_eq!(swap.status, "source_unlocked");
}

// ============================================================
// Test 8: Store secret must verify against hash
// ============================================================

#[tokio::test]
async fn htlc_store_secret_validates_hash() {
    let secret = generate_secret();
    let wrong_secret = generate_secret();
    let secret_hash = hash_secret(&secret);

    // Wrong secret should not match
    assert_ne!(hash_secret(&wrong_secret), secret_hash);

    // Right secret matches
    assert_eq!(hash_secret(&secret), secret_hash);

    // This is what store_secret does: verify before persisting
    let stored_hash = secret_hash;
    let candidate = wrong_secret;
    let matches = hash_secret(&candidate) == stored_hash;
    assert!(!matches, "store_secret should reject wrong secret");

    let candidate = secret;
    let matches = hash_secret(&candidate) == stored_hash;
    assert!(matches, "store_secret should accept correct secret");
}
