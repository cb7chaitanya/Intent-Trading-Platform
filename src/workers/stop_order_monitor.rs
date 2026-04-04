use std::sync::Arc;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus};
use crate::models::intent::{Intent, IntentStatus};
use crate::oracle::service::OracleService;

use tokio::sync::Mutex;

const POLL_INTERVAL_SECS: u64 = 5;

/// Background worker that monitors stop orders and triggers them
/// when the oracle price crosses the stop_price.
pub async fn run(
    pool: PgPool,
    oracle: Arc<OracleService>,
    event_bus: Arc<Mutex<EventBus>>,
    cancel: CancellationToken,
) {
    tracing::info!("Stop order monitor started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("Stop order monitor shutting down");
                return;
            }
        }

        if let Err(e) = check_stop_orders(&pool, &oracle, &event_bus).await {
            tracing::error!(error = %e, "Stop order check failed");
        }
    }
}

async fn check_stop_orders(
    pool: &PgPool,
    oracle: &OracleService,
    event_bus: &Arc<Mutex<EventBus>>,
) -> Result<(), sqlx::Error> {
    // Fetch all pending stop orders
    let stops = sqlx::query_as::<_, Intent>(
        "SELECT * FROM intents
         WHERE order_type = 'stop' AND status = 'open' AND stop_price IS NOT NULL
         ORDER BY created_at ASC
         LIMIT 100",
    )
    .fetch_all(pool)
    .await?;

    if stops.is_empty() {
        return Ok(());
    }

    for intent in stops {
        let stop_price = intent.stop_price.unwrap_or(0);

        // Look up current oracle price for this market
        let prices = oracle.get_all_prices().await;
        let current_price = prices
            .iter()
            .find(|p| {
                // Match market by token pair (simplified: check if any market price exists)
                true
            })
            .map(|p| p.price);

        let Some(price) = current_price else {
            continue;
        };

        // Trigger if price has crossed the stop level
        let triggered = price <= stop_price; // stop-loss: trigger when price falls to stop

        if triggered {
            tracing::info!(
                intent_id = %intent.id,
                stop_price,
                current_price = price,
                "stop_order_triggered"
            );

            // Activate the intent by changing status to Open (which the auction engine picks up)
            // and change order_type to Market so it fills at any price
            sqlx::query(
                "UPDATE intents SET status = 'open', order_type = 'market'
                 WHERE id = $1 AND status = 'open' AND order_type = 'stop'",
            )
            .bind(intent.id)
            .execute(pool)
            .await?;

            // Publish intent_created event so the auction engine picks it up
            let mut triggered_intent = intent.clone();
            triggered_intent.status = IntentStatus::Open;

            let _ = event_bus
                .lock()
                .await
                .publish(&Event::IntentCreated(triggered_intent))
                .await;

            tracing::info!(
                intent_id = %intent.id,
                "stop_order_activated"
            );
        }
    }

    Ok(())
}
