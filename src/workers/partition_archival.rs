use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::config;

const CHECK_INTERVAL_SECS: u64 = 86400; // daily

/// Background worker that archives (detach + drop) old partitions.
pub async fn run(pool: PgPool, cancel: CancellationToken) {
    tracing::info!("Partition archival worker started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("Partition archival worker shutting down");
                return;
            }
        }

        let retention = config::get().partition_retention_months;

        match archive_partitions(&pool, retention).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(archived = count, retention_months = retention, "partitions_archived");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Partition archival failed");
            }
        }
    }
}

async fn archive_partitions(pool: &PgPool, retention_months: i32) -> Result<i32, sqlx::Error> {
    let archived = sqlx::query_scalar::<_, i32>(
        "SELECT archive_old_partitions($1)",
    )
    .bind(retention_months)
    .fetch_one(pool)
    .await?;

    Ok(archived)
}
