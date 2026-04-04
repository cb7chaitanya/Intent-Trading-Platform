use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
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
    retry_count: i32,
}

/// Record a failed settlement for later retry.
pub async fn record_failure(
    pool: &PgPool,
    trade_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    let next_retry = Utc::now() + Duration::seconds(backoff_secs(0));

    sqlx::query(
        "INSERT INTO failed_settlements (id, trade_id, retry_count, last_error, next_retry_at)
         VALUES ($1, $2, 0, $3, $4)
         ON CONFLICT (trade_id) DO UPDATE SET
            last_error = EXCLUDED.last_error,
            next_retry_at = EXCLUDED.next_retry_at",
    )
    .bind(id)
    .bind(trade_id)
    .bind(error)
    .bind(next_retry)
    .execute(pool)
    .await?;

    tracing::warn!(
        trade_id = %trade_id,
        error = error,
        next_retry = %next_retry,
        "settlement_failure_recorded"
    );

    Ok(())
}

/// Background worker that retries failed settlements.
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
        "SELECT id, trade_id, retry_count FROM failed_settlements
         WHERE permanently_failed = FALSE AND next_retry_at <= $1
         ORDER BY next_retry_at ASC
         LIMIT 50",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    if pending.is_empty() {
        return Ok(());
    }

    tracing::info!(count = pending.len(), "Processing settlement retries");

    for entry in pending {
        let attempt = entry.retry_count + 1;

        tracing::info!(
            trade_id = %entry.trade_id,
            attempt = attempt,
            max = MAX_RETRIES,
            "settlement_retry"
        );

        match engine.settle_trade(entry.trade_id).await {
            Ok(_) => {
                // Success — remove from retry queue
                sqlx::query("DELETE FROM failed_settlements WHERE id = $1")
                    .bind(entry.id)
                    .execute(pool)
                    .await?;

                tracing::info!(
                    trade_id = %entry.trade_id,
                    attempt = attempt,
                    "settlement_retry_success"
                );
            }
            Err(super::engine::SettlementError::AlreadySettled) => {
                // Idempotent — trade was already settled, clean up
                sqlx::query("DELETE FROM failed_settlements WHERE id = $1")
                    .bind(entry.id)
                    .execute(pool)
                    .await?;

                tracing::info!(
                    trade_id = %entry.trade_id,
                    "settlement_retry_already_settled"
                );
            }
            Err(e) => {
                let error_msg = e.to_string();

                if attempt >= MAX_RETRIES {
                    // Permanently failed
                    sqlx::query(
                        "UPDATE failed_settlements
                         SET retry_count = $1, last_error = $2, permanently_failed = TRUE
                         WHERE id = $3",
                    )
                    .bind(attempt)
                    .bind(&error_msg)
                    .bind(entry.id)
                    .execute(pool)
                    .await?;

                    counters::SETTLEMENT_FAILURES_TOTAL.inc();

                    tracing::error!(
                        trade_id = %entry.trade_id,
                        attempt = attempt,
                        error = %error_msg,
                        "settlement_permanently_failed"
                    );
                } else {
                    // Schedule next retry with exponential backoff
                    let next = Utc::now() + Duration::seconds(backoff_secs(attempt));

                    sqlx::query(
                        "UPDATE failed_settlements
                         SET retry_count = $1, last_error = $2, next_retry_at = $3
                         WHERE id = $4",
                    )
                    .bind(attempt)
                    .bind(&error_msg)
                    .bind(next)
                    .bind(entry.id)
                    .execute(pool)
                    .await?;

                    tracing::warn!(
                        trade_id = %entry.trade_id,
                        attempt = attempt,
                        next_retry = %next,
                        error = %error_msg,
                        "settlement_retry_failed"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Exponential backoff: 10s, 30s, 90s, 270s, 810s
fn backoff_secs(retry_count: i32) -> i64 {
    10 * 3_i64.pow(retry_count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff_secs(0), 10);    // 10s
        assert_eq!(backoff_secs(1), 30);    // 30s
        assert_eq!(backoff_secs(2), 90);    // 1.5min
        assert_eq!(backoff_secs(3), 270);   // 4.5min
        assert_eq!(backoff_secs(4), 810);   // 13.5min
    }
}
