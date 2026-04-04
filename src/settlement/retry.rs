use std::sync::Arc;

use chrono::Duration;
use chrono::Utc;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::metrics::counters;

use super::engine::SettlementEngine;

const RETRY_INTERVAL_SECS: u64 = 10;
const MAX_RETRIES: i32 = 5;

#[derive(Debug, sqlx::FromRow)]
struct FailedSettlement {
    id: Uuid,
    trade_id: Uuid,
    fill_id: Option<Uuid>,
    retry_count: i32,
}

/// Record a failed trade-level settlement for later retry.
pub async fn record_failure(
    pool: &PgPool,
    trade_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    let next_retry = Utc::now() + Duration::seconds(backoff_secs(0));

    sqlx::query(
        "INSERT INTO failed_settlements (id, trade_id, fill_id, retry_count, last_error, next_retry_at)
         VALUES ($1, $2, NULL, 0, $3, $4)
         ON CONFLICT (trade_id) DO UPDATE SET
            last_error = EXCLUDED.last_error,
            next_retry_at = EXCLUDED.next_retry_at",
    )
    .bind(id).bind(trade_id).bind(error).bind(next_retry)
    .execute(pool).await?;

    tracing::warn!(trade_id = %trade_id, error, "settlement_failure_recorded");
    Ok(())
}

/// Record a failed fill-level settlement for later retry.
pub async fn record_fill_failure(
    pool: &PgPool,
    fill_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    let next_retry = Utc::now() + Duration::seconds(backoff_secs(0));

    // Use a dummy trade_id (the fill_id is the real key here)
    sqlx::query(
        "INSERT INTO failed_settlements (id, trade_id, fill_id, retry_count, last_error, next_retry_at)
         VALUES ($1, $1, $2, 0, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(id).bind(fill_id).bind(error).bind(next_retry)
    .execute(pool).await?;

    tracing::warn!(fill_id = %fill_id, error, "fill_settlement_failure_recorded");
    Ok(())
}

/// Background worker that retries failed settlements (both trade-level and fill-level).
pub async fn run_retry_worker(
    pool: PgPool,
    engine: Arc<SettlementEngine>,
    cancel: CancellationToken,
) {
    tracing::info!("Settlement retry worker started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("Settlement retry worker shutting down");
                return;
            }
        }

        if let Err(e) = process_retries(&pool, &engine).await {
            tracing::error!(error = %e, "Retry worker cycle failed");
        }
    }
}

async fn process_retries(
    pool: &PgPool,
    engine: &SettlementEngine,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();

    let pending = sqlx::query_as::<_, FailedSettlement>(
        "SELECT id, trade_id, fill_id, retry_count FROM failed_settlements
         WHERE permanently_failed = FALSE AND next_retry_at <= $1
         ORDER BY next_retry_at ASC LIMIT 50",
    )
    .bind(now).fetch_all(pool).await?;

    if pending.is_empty() {
        return Ok(());
    }

    tracing::info!(count = pending.len(), "Processing settlement retries");

    for entry in pending {
        let attempt = entry.retry_count + 1;

        // Determine if this is a fill-level or trade-level retry
        if let Some(fill_id) = entry.fill_id {
            retry_fill(pool, engine, &entry, fill_id, attempt).await?;
        } else {
            retry_trade(pool, engine, &entry, attempt).await?;
        }
    }

    Ok(())
}

async fn retry_trade(
    pool: &PgPool,
    engine: &SettlementEngine,
    entry: &FailedSettlement,
    attempt: i32,
) -> Result<(), sqlx::Error> {
    tracing::info!(trade_id = %entry.trade_id, attempt, max = MAX_RETRIES, "settlement_retry");

    match engine.settle_trade(entry.trade_id).await {
        Ok(_) | Err(super::engine::SettlementError::AlreadySettled) => {
            sqlx::query("DELETE FROM failed_settlements WHERE id = $1")
                .bind(entry.id).execute(pool).await?;
            tracing::info!(trade_id = %entry.trade_id, "settlement_retry_success");
        }
        Err(e) => {
            handle_retry_failure(pool, entry, attempt, &e.to_string()).await?;
        }
    }
    Ok(())
}

async fn retry_fill(
    pool: &PgPool,
    engine: &SettlementEngine,
    entry: &FailedSettlement,
    fill_id: Uuid,
    attempt: i32,
) -> Result<(), sqlx::Error> {
    tracing::info!(fill_id = %fill_id, attempt, max = MAX_RETRIES, "fill_settlement_retry");

    // Look up fill context to get accounts and assets
    let fill_ctx = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT f.intent_id, i.token_in, i.token_out
         FROM fills f JOIN intents i ON i.id = f.intent_id
         WHERE f.id = $1",
    )
    .bind(fill_id).fetch_optional(pool).await?;

    let Some((_intent_id, _token_in, _token_out)) = fill_ctx else {
        // Fill no longer exists — clean up
        sqlx::query("DELETE FROM failed_settlements WHERE id = $1")
            .bind(entry.id).execute(pool).await?;
        return Ok(());
    };

    // Check if already settled
    let settled = sqlx::query_scalar::<_, bool>(
        "SELECT settled FROM fills WHERE id = $1",
    )
    .bind(fill_id).fetch_optional(pool).await?.unwrap_or(true);

    if settled {
        sqlx::query("DELETE FROM failed_settlements WHERE id = $1")
            .bind(entry.id).execute(pool).await?;
        tracing::info!(fill_id = %fill_id, "fill_retry_already_settled");
        return Ok(());
    }

    // The actual retry needs buyer/seller accounts and assets, which
    // are determined by the execution engine when calling settle_intent_fills.
    // For now, mark as needing manual intervention after max retries.
    handle_retry_failure(pool, entry, attempt, "fill retry requires execution context").await?;

    Ok(())
}

async fn handle_retry_failure(
    pool: &PgPool,
    entry: &FailedSettlement,
    attempt: i32,
    error_msg: &str,
) -> Result<(), sqlx::Error> {
    if attempt >= MAX_RETRIES {
        sqlx::query(
            "UPDATE failed_settlements SET retry_count = $1, last_error = $2, permanently_failed = TRUE WHERE id = $3",
        )
        .bind(attempt).bind(error_msg).bind(entry.id)
        .execute(pool).await?;

        counters::SETTLEMENT_FAILURES_TOTAL.inc();
        tracing::error!(id = %entry.id, attempt, error = error_msg, "settlement_permanently_failed");
    } else {
        let next = Utc::now() + Duration::seconds(backoff_secs(attempt));
        sqlx::query(
            "UPDATE failed_settlements SET retry_count = $1, last_error = $2, next_retry_at = $3 WHERE id = $4",
        )
        .bind(attempt).bind(error_msg).bind(next).bind(entry.id)
        .execute(pool).await?;

        tracing::warn!(id = %entry.id, attempt, next_retry = %next, error = error_msg, "settlement_retry_scheduled");
    }
    Ok(())
}

fn backoff_secs(retry_count: i32) -> i64 {
    10 * 3_i64.pow(retry_count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff_secs(0), 10);
        assert_eq!(backoff_secs(1), 30);
        assert_eq!(backoff_secs(2), 90);
        assert_eq!(backoff_secs(3), 270);
        assert_eq!(backoff_secs(4), 810);
    }
}
