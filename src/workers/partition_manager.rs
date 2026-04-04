use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

const CHECK_INTERVAL_SECS: u64 = 86400; // daily
const MONTHS_AHEAD: i32 = 3;

/// Background worker that ensures partitions exist for upcoming months.
pub async fn run(pool: PgPool, cancel: CancellationToken) {
    tracing::info!("Partition manager started");

    // Run immediately on startup
    if let Err(e) = ensure_partitions(&pool).await {
        tracing::error!(error = %e, "Initial partition creation failed");
    }

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("Partition manager shutting down");
                return;
            }
        }

        if let Err(e) = ensure_partitions(&pool).await {
            tracing::error!(error = %e, "Partition creation failed");
        }
    }
}

async fn ensure_partitions(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT create_monthly_partitions($1)")
        .bind(MONTHS_AHEAD)
        .execute(pool)
        .await?;

    tracing::info!(months_ahead = MONTHS_AHEAD, "Partitions verified");
    Ok(())
}
