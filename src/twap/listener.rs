use std::sync::Arc;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::db::stream_bus::{StreamBus, STREAM_INTENT_SETTLED};
use crate::settlement::worker::IntentSettledEvent;

use super::model::TwapChildIntent;
use super::service::TwapService;

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
        Err(_) => return,
    };

    // Check if this intent is a TWAP child
    let child = match sqlx::query_as::<_, TwapChildIntent>(
        "SELECT * FROM twap_child_intents WHERE intent_id = $1",
    )
    .bind(event.intent_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return, // Not a TWAP child
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
        status = %event.status,
        slice = child.slice_index,
        "twap_child_event"
    );

    match event.status.as_str() {
        "Completed" => {
            if let Err(e) = twap_service
                .record_child_completed(child.twap_id, child.id, event.settled_qty)
                .await
            {
                tracing::error!(twap_id = %child.twap_id, error = %e, "twap_record_completed_failed");
            }
        }
        "PartiallyFilled" => {
            // PartiallyFilled means some fills settled but not all.
            // Update the TWAP with whatever has been settled so far.
            // The next event (Completed or Expired) will finalize.
            tracing::info!(
                twap_id = %child.twap_id,
                intent_id = %event.intent_id,
                settled_qty = event.settled_qty,
                "twap_child_partially_filled"
            );
            // Don't increment slices_completed yet — wait for final state
        }
        "Expired" => {
            if let Err(e) = twap_service
                .record_child_expired(child.twap_id, child.id)
                .await
            {
                tracing::error!(twap_id = %child.twap_id, error = %e, "twap_record_expired_failed");
            }
        }
        other => {
            tracing::debug!(status = other, "Ignoring unhandled intent status for TWAP");
        }
    }
}
