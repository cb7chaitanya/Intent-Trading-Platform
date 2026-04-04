use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::db::redis::{Event, EventBus};
use crate::models::intent::{Intent, IntentStatus, OrderType};
use crate::oracle::service::OracleService;

const POLL_INTERVAL_SECS: u64 = 5;

/// Background worker that monitors stop orders and triggers them
/// when the oracle price crosses the stop_price.
///
/// Supports:
/// - Stop-loss sell: triggers when price falls to/below stop_price
/// - Stop-buy: triggers when price rises to/above stop_price
/// - Stop-limit: triggered stop converts to limit order (if limit_price set)
/// - Trigger-once guarantee via `triggered_at IS NULL` atomic UPDATE
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
    // Fetch all pending stop orders that haven't been triggered yet
    let stops = sqlx::query_as::<_, Intent>(
        "SELECT * FROM intents
         WHERE order_type = 'stop'
           AND status = 'open'
           AND stop_price IS NOT NULL
           AND triggered_at IS NULL
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
        let side = intent.stop_side.as_deref().unwrap_or("sell");

        // Look up market price via the markets + market_prices tables
        // Intent stores token_in/token_out as strings matching asset_type enum
        let price_row: Option<(i64,)> = sqlx::query_as(
            "SELECT mp.price
             FROM market_prices mp
             JOIN markets m ON m.id = mp.market_id
             WHERE UPPER(m.base_asset::text) = UPPER($1)
               AND UPPER(m.quote_asset::text) = UPPER($2)
             LIMIT 1",
        )
        .bind(&intent.token_in)
        .bind(&intent.token_out)
        .fetch_optional(pool)
        .await?;

        let Some((current_price,)) = price_row else {
            tracing::debug!(
                intent_id = %intent.id,
                token_in = %intent.token_in,
                token_out = %intent.token_out,
                "no_oracle_price_for_stop_order"
            );
            continue;
        };

        // Check trigger condition based on stop side
        let triggered = match side {
            "buy" => current_price >= stop_price,  // stop-buy: price rises to/above stop
            _     => current_price <= stop_price,  // stop-loss sell: price falls to/below stop
        };

        if !triggered {
            continue;
        }

        tracing::info!(
            intent_id = %intent.id,
            stop_price,
            current_price,
            side,
            "stop_order_triggered"
        );

        // Determine conversion: stop-limit → Limit, else → Market
        let (new_order_type, new_status) = if intent.limit_price.is_some() {
            (OrderType::Limit, IntentStatus::Open)
        } else {
            (OrderType::Market, IntentStatus::Open)
        };

        let new_type_str = match new_order_type {
            OrderType::Limit => "limit",
            OrderType::Market => "market",
            OrderType::Stop => unreachable!(),
        };

        // Atomic trigger-once: only UPDATE if triggered_at IS NULL
        let result = sqlx::query(
            "UPDATE intents
             SET status = 'open',
                 order_type = $2::order_type,
                 triggered_at = NOW()
             WHERE id = $1
               AND order_type = 'stop'
               AND triggered_at IS NULL",
        )
        .bind(intent.id)
        .bind(new_type_str)
        .execute(pool)
        .await?;

        // If rows_affected == 0, another instance already triggered it
        if result.rows_affected() == 0 {
            tracing::debug!(
                intent_id = %intent.id,
                "stop_order_already_triggered"
            );
            continue;
        }

        // Publish StopTriggered event
        let mut triggered_intent = intent.clone();
        triggered_intent.status = new_status;
        triggered_intent.order_type = new_order_type.clone();
        triggered_intent.triggered_at = Some(chrono::Utc::now());

        let _ = event_bus
            .lock()
            .await
            .publish(&Event::StopTriggered(triggered_intent.clone()))
            .await;

        // Also publish IntentCreated so auction engine picks it up
        let _ = event_bus
            .lock()
            .await
            .publish(&Event::IntentCreated(triggered_intent))
            .await;

        tracing::info!(
            intent_id = %intent.id,
            new_order_type = new_type_str,
            "stop_order_activated"
        );
    }

    Ok(())
}
