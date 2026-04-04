use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::stream_bus::{StreamBus, STREAM_INTENT_SETTLED};
use crate::settlement::worker::IntentSettledEvent;

use super::model::{TwapChildIntent, TwapStatus};
use super::service::TwapService;

/// Background worker that listens for intent.settled events and updates
/// TWAP parent progress when a child intent completes settlement.
pub async fn run(
    stream_bus: Arc<StreamBus>,
    twap_service: Arc<TwapService>,
    pool: PgPool,
    cancel: CancellationToken,
) {
    tracing::info!("TWAP completion listener started");

    let streams = &[STREAM_INTENT_SETTLED];
    let group = "twap-listener";
    let consumer = "listener-1";

    for s in streams {
        if let Err(e) = stream_bus.ensure_group(s, group).await {
            tracing::error!(stream = s, error = %e, "Failed to create TWAP consumer group");
        }
    }

    loop {
        tokio::select! {
            result = stream_bus.subscribe(streams, group, consumer, |event| {
                let twap_svc = Arc::clone(&twap_service);
                let pool = pool.clone();
                async move {
                    process_intent_settled(&twap_svc, &pool, &event.payload).await;
                }
            }) => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "TWAP listener stream error");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("TWAP completion listener shutting down");
                return;
            }
        }
    }
}

async fn process_intent_settled(
    twap_service: &TwapService,
    pool: &PgPool,
    payload: &str,
) {
    let event: IntentSettledEvent = match serde_json::from_str(payload) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(error = %e, "Not a valid IntentSettledEvent (may not be TWAP-related)");
            return;
        }
    };

    // Check if this intent is a TWAP child
    let child = sqlx::query_as::<_, TwapChildIntent>(
        "SELECT * FROM twap_child_intents WHERE intent_id = $1",
    )
    .bind(event.intent_id)
    .fetch_optional(pool)
    .await;

    let child = match child {
        Ok(Some(c)) => c,
        Ok(None) => return, // Not a TWAP child — ignore
        Err(e) => {
            tracing::error!(error = %e, "Failed to look up TWAP child");
            return;
        }
    };

    tracing::info!(
        twap_id = %child.twap_id,
        child_id = %child.id,
        intent_id = %event.intent_id,
        settled_qty = event.settled_qty,
        slice = child.slice_index,
        "twap_child_settled"
    );

    // Record child completion in TWAP service
    if let Err(e) = twap_service
        .record_child_completed(child.twap_id, child.id, event.settled_qty)
        .await
    {
        tracing::error!(
            twap_id = %child.twap_id,
            error = %e,
            "Failed to record TWAP child completion"
        );
        return;
    }

    // Check for TWAP expiry (deadline passed with remaining qty)
    check_twap_expiry(pool, child.twap_id).await;
}

async fn check_twap_expiry(pool: &PgPool, twap_id: Uuid) {
    let result = sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT duration_secs, EXTRACT(EPOCH FROM created_at)::BIGINT, status::text
         FROM twap_intents WHERE id = $1",
    )
    .bind(twap_id)
    .fetch_optional(pool)
    .await;

    let (duration, created_epoch, status) = match result {
        Ok(Some(r)) => r,
        _ => return,
    };

    if status != "active" {
        return;
    }

    let deadline = created_epoch + duration;
    let now = Utc::now().timestamp();

    if now > deadline {
        // Check if there's remaining unfilled qty
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT total_qty - filled_qty FROM twap_intents WHERE id = $1",
        )
        .bind(twap_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0);

        if remaining > 0 {
            let _ = sqlx::query(
                "UPDATE twap_intents SET status = 'failed', finished_at = NOW() WHERE id = $1 AND status = 'active'",
            )
            .bind(twap_id)
            .execute(pool)
            .await;

            // Cancel remaining pending children
            let _ = sqlx::query(
                "UPDATE twap_child_intents SET status = 'expired' WHERE twap_id = $1 AND status = 'pending'",
            )
            .bind(twap_id)
            .execute(pool)
            .await;

            tracing::warn!(
                twap_id = %twap_id,
                remaining_qty = remaining,
                "twap_expired_with_remaining"
            );
        }
    }
}
