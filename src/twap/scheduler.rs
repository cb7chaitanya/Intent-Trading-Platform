use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::services::intent_service::IntentService;

use super::model::TwapChildIntent;
use super::service::TwapService;

const POLL_INTERVAL_SECS: u64 = 5;

/// Background worker that submits scheduled TWAP child intents.
pub async fn run(
    pool: PgPool,
    intent_service: Arc<Mutex<IntentService>>,
    twap_service: Arc<TwapService>,
    cancel: CancellationToken,
) {
    tracing::info!("TWAP scheduler started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("TWAP scheduler shutting down");
                return;
            }
        }

        if let Err(e) = process_due_slices(&pool, &intent_service, &twap_service).await {
            tracing::error!(error = %e, "TWAP scheduler cycle failed");
        }
    }
}

async fn process_due_slices(
    pool: &PgPool,
    intent_service: &Arc<Mutex<IntentService>>,
    twap_service: &TwapService,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();

    // Find child intents that are due and whose parent is still active
    let due = sqlx::query_as::<_, TwapChildIntent>(
        "SELECT c.* FROM twap_child_intents c
         JOIN twap_intents t ON t.id = c.twap_id
         WHERE c.status = 'pending'
         AND c.scheduled_at <= $1
         AND t.status = 'active'
         ORDER BY c.scheduled_at ASC
         LIMIT 20",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    if due.is_empty() {
        return Ok(());
    }

    tracing::info!(count = due.len(), "Processing due TWAP slices");

    for child in due {
        // Look up parent for context
        let parent = sqlx::query_as::<_, super::model::TwapIntent>(
            "SELECT * FROM twap_intents WHERE id = $1",
        )
        .bind(child.twap_id)
        .fetch_optional(pool)
        .await?;

        let Some(parent) = parent else { continue };

        // Mark as submitted
        sqlx::query("UPDATE twap_child_intents SET status = 'submitted' WHERE id = $1")
            .bind(child.id)
            .execute(pool)
            .await?;

        // Calculate deadline: scheduled_at + interval_secs
        let deadline = child.scheduled_at.timestamp() + parent.interval_secs;

        // Submit the actual intent
        let mut svc = intent_service.lock().await;
        let result = svc
            .create_intent(
                parent.user_id.clone(),
                parent.account_id,
                parent.token_in.clone(),
                parent.token_out.clone(),
                child.qty as u64,
                parent.min_price as u64,
                deadline,
            )
            .await;

        match result {
            Ok(intent) => {
                // Update child with actual intent_id
                sqlx::query("UPDATE twap_child_intents SET intent_id = $1 WHERE id = $2")
                    .bind(intent.id)
                    .bind(child.id)
                    .execute(pool)
                    .await?;

                tracing::info!(
                    twap_id = %child.twap_id,
                    slice = child.slice_index,
                    intent_id = %intent.id,
                    qty = child.qty,
                    "twap_slice_submitted"
                );
            }
            Err(e) => {
                tracing::error!(
                    twap_id = %child.twap_id,
                    slice = child.slice_index,
                    error = %e,
                    "twap_slice_submit_failed"
                );

                let _ = twap_service.record_child_failed(child.id).await;
            }
        }
    }

    Ok(())
}
