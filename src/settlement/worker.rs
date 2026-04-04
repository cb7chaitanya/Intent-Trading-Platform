use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::balances::model::Asset;
use crate::db::stream_bus::{StreamBus, STREAM_EXECUTION_COMPLETED};
use crate::models::intent::IntentStatus;

use super::engine::SettlementEngine;
use super::retry;

/// Payload published by ExecutionEngine when a fill's execution completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCompletedEvent {
    pub execution_id: Uuid,
    pub fill_id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,
    pub buyer_account_id: Uuid,
    pub seller_account_id: Uuid,
    pub token_in: String,
    pub token_out: String,
    pub fee_rate: f64,
}

/// Background worker that consumes execution.completed events and triggers settlement.
pub async fn run(
    stream_bus: Arc<StreamBus>,
    settlement: Arc<SettlementEngine>,
    pool: PgPool,
    cancel: tokio_util::sync::CancellationToken,
) {
    tracing::info!("Settlement worker started — listening for execution.completed events");

    let streams = &[STREAM_EXECUTION_COMPLETED];
    let group = "settlement-worker";
    let consumer = "worker-1";

    // Ensure consumer group exists
    for s in streams {
        if let Err(e) = stream_bus.ensure_group(s, group).await {
            tracing::error!(stream = s, error = %e, "Failed to create consumer group");
        }
    }

    loop {
        tokio::select! {
            result = stream_bus.subscribe(streams, group, consumer, |event| {
                let settlement = Arc::clone(&settlement);
                let pool = pool.clone();
                async move {
                    process_event(&settlement, &pool, &event.payload).await;
                }
            }) => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "Settlement worker stream error");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Settlement worker shutting down");
                return;
            }
        }
    }
}

async fn process_event(
    settlement: &SettlementEngine,
    pool: &PgPool,
    payload: &str,
) {
    let event: ExecutionCompletedEvent = match serde_json::from_str(payload) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse ExecutionCompletedEvent");
            return;
        }
    };

    tracing::info!(
        fill_id = %event.fill_id,
        intent_id = %event.intent_id,
        execution_id = %event.execution_id,
        "settlement_worker_received"
    );

    let asset_in = match parse_asset(&event.token_in) {
        Some(a) => a,
        None => {
            tracing::error!(token = %event.token_in, "Unknown asset in settlement event");
            return;
        }
    };
    let asset_out = match parse_asset(&event.token_out) {
        Some(a) => a,
        None => {
            tracing::error!(token = %event.token_out, "Unknown asset in settlement event");
            return;
        }
    };

    // Settle the fill (idempotent — AlreadySettled is not an error)
    match settlement
        .settle_fill(
            event.fill_id,
            event.buyer_account_id,
            event.seller_account_id,
            &asset_in,
            &asset_out,
            event.fee_rate,
        )
        .await
    {
        Ok(()) => {
            tracing::info!(
                fill_id = %event.fill_id,
                intent_id = %event.intent_id,
                "fill_auto_settled"
            );
        }
        Err(super::engine::SettlementError::AlreadySettled) => {
            tracing::debug!(fill_id = %event.fill_id, "fill_already_settled");
        }
        Err(e) => {
            tracing::error!(
                fill_id = %event.fill_id,
                error = %e,
                "fill_auto_settlement_failed"
            );
            let _ = retry::record_fill_failure(pool, event.fill_id, &e.to_string()).await;
            return;
        }
    }

    // After settling this fill, recompute intent status
    update_intent_status(settlement, event.intent_id).await;
}

async fn update_intent_status(settlement: &SettlementEngine, intent_id: Uuid) {
    let pool = settlement.pool();

    let intent_amount = match sqlx::query_scalar::<_, i64>(
        "SELECT amount_in FROM intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(a)) => a,
        _ => return,
    };

    let settled_qty = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(filled_qty), 0) FROM fills WHERE intent_id = $1 AND settled = TRUE",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let new_status = if settled_qty >= intent_amount {
        IntentStatus::Completed
    } else if settled_qty > 0 {
        IntentStatus::PartiallyFilled
    } else {
        return; // no change needed
    };

    let _ = sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
        .bind(&new_status)
        .bind(intent_id)
        .execute(pool)
        .await;

    tracing::info!(
        intent_id = %intent_id,
        settled_qty,
        intent_amount,
        status = ?new_status,
        "intent_status_auto_updated"
    );
}

fn parse_asset(s: &str) -> Option<Asset> {
    match s.to_uppercase().as_str() {
        "USDC" => Some(Asset::USDC),
        "ETH" => Some(Asset::ETH),
        "BTC" => Some(Asset::BTC),
        "SOL" => Some(Asset::SOL),
        _ => None,
    }
}
