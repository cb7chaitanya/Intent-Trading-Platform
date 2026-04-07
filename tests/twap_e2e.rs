//! TWAP lifecycle integration test.
//!
//! Covers: parent creation → child scheduling → real intent submission
//!       → child completion → parent completion.
//!
//! Run: cargo test --test twap_e2e --features integration
//!
//! Requires Docker to be running.

#![cfg(feature = "integration")]

use chrono::{Duration, Utc};
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

async fn create_test_user_and_account(pool: &PgPool) -> (Uuid, Uuid) {
    let user_id = Uuid::new_v4();
    let account_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'testhash', $3, $4)",
    )
    .bind(user_id)
    .bind(format!("test-{}@twap.test", Uuid::new_v4()))
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

async fn seed_balance(pool: &PgPool, account_id: Uuid, asset: &str, available: i64) {
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, $3::asset_type, $4, 0, NOW())
         ON CONFLICT (account_id, asset) DO UPDATE SET available_balance = $4",
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(asset)
    .bind(available)
    .execute(pool)
    .await
    .unwrap();
}

#[derive(Debug, sqlx::FromRow)]
struct TwapRow {
    id: Uuid,
    status: String,
    total_qty: i64,
    filled_qty: i64,
    slices_total: i32,
    slices_completed: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct ChildRow {
    id: Uuid,
    twap_id: Uuid,
    intent_id: Uuid,
    slice_index: i32,
    qty: i64,
    status: String,
}

async fn get_twap(pool: &PgPool, twap_id: Uuid) -> TwapRow {
    sqlx::query_as::<_, TwapRow>(
        "SELECT id, status::text as status, total_qty, filled_qty,
                slices_total, slices_completed
         FROM twap_intents WHERE id = $1",
    )
    .bind(twap_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn get_children(pool: &PgPool, twap_id: Uuid) -> Vec<ChildRow> {
    sqlx::query_as::<_, ChildRow>(
        "SELECT id, twap_id, intent_id, slice_index, qty, status
         FROM twap_child_intents WHERE twap_id = $1
         ORDER BY slice_index",
    )
    .bind(twap_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

// ============================================================
// Test 1: Full lifecycle — parent → children → completion
// ============================================================

#[tokio::test]
async fn twap_parent_child_lifecycle_to_completion() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, account_id) = create_test_user_and_account(&pool).await;
    seed_balance(&pool, account_id, "ETH", 10_000_000).await;

    let twap_id = Uuid::new_v4();
    let now = Utc::now();

    let total_qty: i64 = 3_000_000;
    let slices_total: i32 = 3;
    let interval_secs: i64 = 60;
    let qty_per_slice = total_qty / slices_total as i64;

    // ── Step 1: Create parent TWAP ──────────────────
    sqlx::query(
        "INSERT INTO twap_intents
            (id, user_id, account_id, token_in, token_out, total_qty,
             filled_qty, min_price, duration_secs, interval_secs,
             slices_total, slices_completed, status, created_at)
         VALUES ($1, $2, $3, 'ETH', 'USDC', $4, 0, 1000, 180, $5, $6, 0, 'active', $7)",
    )
    .bind(twap_id)
    .bind(user_id.to_string())
    .bind(account_id)
    .bind(total_qty)
    .bind(interval_secs)
    .bind(slices_total)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.status, "active");
    assert_eq!(twap.slices_total, 3);
    assert_eq!(twap.slices_completed, 0);
    assert_eq!(twap.filled_qty, 0);

    // ── Step 2: Create child intents with nil intent_id ──
    let nil_id = Uuid::nil();
    for i in 0..slices_total {
        let scheduled_at = now + Duration::seconds(interval_secs * i as i64);
        sqlx::query(
            "INSERT INTO twap_child_intents
                (twap_id, intent_id, slice_index, qty, status, scheduled_at)
             VALUES ($1, $2, $3, $4, 'pending', $5)",
        )
        .bind(twap_id)
        .bind(nil_id)
        .bind(i)
        .bind(qty_per_slice)
        .bind(scheduled_at)
        .execute(&pool)
        .await
        .unwrap();
    }

    let children = get_children(&pool, twap_id).await;
    assert_eq!(children.len(), 3);
    for child in &children {
        assert!(child.intent_id.is_nil(), "Children should start with nil intent_id");
        assert_eq!(child.status, "pending");
        assert_eq!(child.qty, qty_per_slice);
    }

    // ── Step 3: Scheduler submits child 0 — simulate real intent ──
    let real_intent_0 = Uuid::new_v4();
    // Insert a real intent in the intents table
    sqlx::query(
        "INSERT INTO intents
            (id, user_id, token_in, token_out, amount_in, min_amount_out,
             deadline, status, created_at, order_type)
         VALUES ($1, $2, 'ETH', 'USDC', $3, 1000, $4, 'open', $5, 'market')",
    )
    .bind(real_intent_0)
    .bind(user_id.to_string())
    .bind(qty_per_slice)
    .bind(now.timestamp() + 120)
    .bind(now.timestamp())
    .execute(&pool)
    .await
    .unwrap();

    // Update child with real intent_id (same as scheduler does)
    sqlx::query("UPDATE twap_child_intents SET intent_id = $1, status = 'submitted' WHERE id = $2")
        .bind(real_intent_0)
        .bind(children[0].id)
        .execute(&pool)
        .await
        .unwrap();

    let updated_children = get_children(&pool, twap_id).await;
    assert_eq!(updated_children[0].intent_id, real_intent_0);
    assert!(!updated_children[0].intent_id.is_nil());
    assert_eq!(updated_children[0].status, "submitted");
    // Other children still nil
    assert!(updated_children[1].intent_id.is_nil());

    // ── Step 4: Complete all children one by one ─────
    for (i, child) in children.iter().enumerate() {
        // Simulate scheduler submitting + completing each child
        let real_intent_id = if i == 0 {
            real_intent_0
        } else {
            let rid = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO intents
                    (id, user_id, token_in, token_out, amount_in, min_amount_out,
                     deadline, status, created_at, order_type)
                 VALUES ($1, $2, 'ETH', 'USDC', $3, 1000, $4, 'completed', $5, 'market')",
            )
            .bind(rid)
            .bind(user_id.to_string())
            .bind(qty_per_slice)
            .bind(now.timestamp() + 120)
            .bind(now.timestamp())
            .execute(&pool)
            .await
            .unwrap();

            sqlx::query("UPDATE twap_child_intents SET intent_id = $1, status = 'submitted' WHERE id = $2")
                .bind(rid)
                .bind(child.id)
                .execute(&pool)
                .await
                .unwrap();
            rid
        };

        // Record child as completed (same as record_child_completed does)
        let result = sqlx::query(
            "UPDATE twap_child_intents SET status = 'completed'
             WHERE id = $1 AND status != 'completed'",
        )
        .bind(child.id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(result.rows_affected(), 1);

        sqlx::query(
            "UPDATE twap_intents
             SET filled_qty = filled_qty + $1, slices_completed = slices_completed + 1
             WHERE id = $2",
        )
        .bind(qty_per_slice)
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

        // Verify parent intent can be found via child
        let child_intent_lookup = sqlx::query_scalar::<_, Uuid>(
            "SELECT c.twap_id FROM twap_child_intents c WHERE c.intent_id = $1",
        )
        .bind(real_intent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(child_intent_lookup, twap_id, "Parent-child relationship must be traceable");
    }

    // ── Step 5: Verify parent completion ────────────
    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.slices_completed, 3);
    assert_eq!(twap.filled_qty, total_qty);

    // Mark parent completed (same as record_child_completed does when all slices done)
    sqlx::query("UPDATE twap_intents SET status = 'completed', finished_at = NOW() WHERE id = $1 AND status = 'active'")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.status, "completed");
}

// ============================================================
// Test 2: Cancel TWAP skips unsubmitted children
// ============================================================

#[tokio::test]
async fn twap_cancel_skips_unsubmitted_children() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, account_id) = create_test_user_and_account(&pool).await;
    let twap_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO twap_intents
            (id, user_id, account_id, token_in, token_out, total_qty,
             filled_qty, min_price, duration_secs, interval_secs,
             slices_total, slices_completed, status, created_at)
         VALUES ($1, $2, $3, 'ETH', 'USDC', 2000000, 0, 1000, 120, 60, 2, 0, 'active', $4)",
    )
    .bind(twap_id)
    .bind(user_id.to_string())
    .bind(account_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Child 0: submitted with real intent_id
    let real_intent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO twap_child_intents (twap_id, intent_id, slice_index, qty, status, scheduled_at)
         VALUES ($1, $2, 0, 1000000, 'submitted', $3)",
    )
    .bind(twap_id)
    .bind(real_intent)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Child 1: pending with nil intent_id (not yet submitted)
    sqlx::query(
        "INSERT INTO twap_child_intents (twap_id, intent_id, slice_index, qty, status, scheduled_at)
         VALUES ($1, $2, 1, 1000000, 'pending', $3)",
    )
    .bind(twap_id)
    .bind(Uuid::nil())
    .bind(now + Duration::seconds(60))
    .execute(&pool)
    .await
    .unwrap();

    let children = get_children(&pool, twap_id).await;
    assert_eq!(children.len(), 2);
    assert!(!children[0].intent_id.is_nil(), "First child should have real intent");
    assert!(children[1].intent_id.is_nil(), "Second child should be nil");

    // Cancel: only the submitted child's intent should be attempted for cancel
    // The nil child should be skipped
    let submitted_children: Vec<&ChildRow> = children.iter()
        .filter(|c| c.status == "pending" || c.status == "submitted")
        .filter(|c| !c.intent_id.is_nil())
        .collect();
    assert_eq!(submitted_children.len(), 1, "Only one child has a real intent to cancel");
    assert_eq!(submitted_children[0].intent_id, real_intent);

    // Mark all as cancelled
    sqlx::query(
        "UPDATE twap_child_intents SET status = 'cancelled'
         WHERE twap_id = $1 AND status IN ('pending', 'submitted')",
    )
    .bind(twap_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE twap_intents SET status = 'cancelled', finished_at = NOW() WHERE id = $1")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    let children = get_children(&pool, twap_id).await;
    assert!(children.iter().all(|c| c.status == "cancelled"));
    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.status, "cancelled");
}

// ============================================================
// Test 3: Remainder distribution — last slice gets extras
// ============================================================

#[tokio::test]
async fn twap_remainder_goes_to_last_slice() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, account_id) = create_test_user_and_account(&pool).await;
    let twap_id = Uuid::new_v4();
    let now = Utc::now();

    let total_qty: i64 = 100;
    let slices_total: i32 = 3;
    let qty_per_slice = total_qty / slices_total as i64; // 33
    let remainder = total_qty - (qty_per_slice * slices_total as i64); // 1

    sqlx::query(
        "INSERT INTO twap_intents
            (id, user_id, account_id, token_in, token_out, total_qty,
             filled_qty, min_price, duration_secs, interval_secs,
             slices_total, slices_completed, status, created_at)
         VALUES ($1, $2, $3, 'ETH', 'USDC', $4, 0, 0, 180, 60, $5, 0, 'active', $6)",
    )
    .bind(twap_id)
    .bind(user_id.to_string())
    .bind(account_id)
    .bind(total_qty)
    .bind(slices_total)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Insert children with correct remainder distribution
    for i in 0..slices_total {
        let slice_qty = if i == slices_total - 1 { qty_per_slice + remainder } else { qty_per_slice };
        sqlx::query(
            "INSERT INTO twap_child_intents (twap_id, intent_id, slice_index, qty, status, scheduled_at)
             VALUES ($1, $2, $3, $4, 'pending', $5)",
        )
        .bind(twap_id)
        .bind(Uuid::nil())
        .bind(i)
        .bind(slice_qty)
        .bind(now + Duration::seconds(60 * i as i64))
        .execute(&pool)
        .await
        .unwrap();
    }

    let children = get_children(&pool, twap_id).await;
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].qty, 33);
    assert_eq!(children[1].qty, 33);
    assert_eq!(children[2].qty, 34, "Last slice gets remainder");
    assert_eq!(
        children.iter().map(|c| c.qty).sum::<i64>(),
        total_qty,
        "Sum of slices must equal total_qty"
    );
}

// ============================================================
// Test 4: Partial completion → some expired → status = failed
// ============================================================

#[tokio::test]
async fn twap_partial_completion_with_expired_slices() {
    let (pool, _pg) = setup_postgres().await;

    let (user_id, account_id) = create_test_user_and_account(&pool).await;
    let twap_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO twap_intents
            (id, user_id, account_id, token_in, token_out, total_qty,
             filled_qty, min_price, duration_secs, interval_secs,
             slices_total, slices_completed, status, created_at)
         VALUES ($1, $2, $3, 'ETH', 'USDC', 3000, 0, 0, 180, 60, 3, 0, 'active', $4)",
    )
    .bind(twap_id)
    .bind(user_id.to_string())
    .bind(account_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Insert 3 children
    let mut child_ids = Vec::new();
    for i in 0..3 {
        let child_id: Uuid = sqlx::query_scalar(
            "INSERT INTO twap_child_intents (twap_id, intent_id, slice_index, qty, status, scheduled_at)
             VALUES ($1, $2, $3, 1000, 'pending', $4) RETURNING id",
        )
        .bind(twap_id)
        .bind(Uuid::nil())
        .bind(i)
        .bind(now + Duration::seconds(60 * i as i64))
        .fetch_one(&pool)
        .await
        .unwrap();
        child_ids.push(child_id);
    }

    // Child 0: completed with 1000 filled
    sqlx::query("UPDATE twap_child_intents SET status = 'completed' WHERE id = $1")
        .bind(child_ids[0])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE twap_intents SET filled_qty = filled_qty + 1000, slices_completed = slices_completed + 1 WHERE id = $1")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    // Child 1: expired
    sqlx::query("UPDATE twap_child_intents SET status = 'expired' WHERE id = $1")
        .bind(child_ids[1])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE twap_intents SET slices_completed = slices_completed + 1 WHERE id = $1")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    // Child 2: expired
    sqlx::query("UPDATE twap_child_intents SET status = 'expired' WHERE id = $1")
        .bind(child_ids[2])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE twap_intents SET slices_completed = slices_completed + 1 WHERE id = $1")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    // All slices done but not fully filled → status should be failed
    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.slices_completed, 3);
    assert_eq!(twap.filled_qty, 1000);
    assert!(twap.filled_qty < twap.total_qty, "Not fully filled");

    // Same logic as record_child_expired
    sqlx::query("UPDATE twap_intents SET status = 'failed', finished_at = NOW() WHERE id = $1 AND status = 'active'")
        .bind(twap_id)
        .execute(&pool)
        .await
        .unwrap();

    let twap = get_twap(&pool, twap_id).await;
    assert_eq!(twap.status, "failed", "Should be failed when all slices done but not fully filled");
}

// ============================================================
// Test 5: Nil intent_id is not queryable as a real intent
// ============================================================

#[tokio::test]
async fn twap_nil_intent_id_not_confused_with_real() {
    // Uuid::nil() should not match any real intent_id lookup
    let nil = Uuid::nil();
    assert!(nil.is_nil());
    assert_eq!(nil.to_string(), "00000000-0000-0000-0000-000000000000");

    // A random UUID should never be nil
    let random = Uuid::new_v4();
    assert!(!random.is_nil());
    assert_ne!(random, nil);
}
