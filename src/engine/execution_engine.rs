use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus, INTENT_MATCHED};
use crate::db::storage::Storage;
use crate::db::stream_bus::{StreamBus, STREAM_EXECUTION_COMPLETED};
use crate::metrics::{counters, histograms};
use crate::models::execution::{Execution, ExecutionStatus};
use crate::models::intent::IntentStatus;
use crate::settlement::worker::ExecutionCompletedEvent;

pub struct ExecutionEngine {
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    stream_bus: Arc<StreamBus>,
    execution_duration_secs: u64,
}

impl ExecutionEngine {
    pub fn new(
        storage: Arc<Storage>,
        event_bus: EventBus,
        stream_bus: Arc<StreamBus>,
        execution_duration_secs: u64,
    ) -> Self {
        Self {
            storage,
            event_bus: Arc::new(Mutex::new(event_bus)),
            stream_bus,
            execution_duration_secs,
        }
    }

    pub async fn start(&self) -> Result<(), redis::RedisError> {
        let mut pubsub = {
            let bus = self.event_bus.lock().await;
            bus.client().get_async_pubsub().await?
        };

        pubsub.subscribe(INTENT_MATCHED).await?;

        let mut stream = pubsub.on_message();
        while let Some(msg) = stream.next().await {
            let payload: String = match msg.get_payload() {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to read execution message payload");
                    continue;
                }
            };

            let event = match serde_json::from_str::<Event>(&payload) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to deserialize execution event");
                    continue;
                }
            };

            if let Event::IntentMatched { intent, bid: _ } = event {
                let storage = Arc::clone(&self.storage);
                let event_bus = Arc::clone(&self.event_bus);
                let stream_bus = Arc::clone(&self.stream_bus);
                let duration = self.execution_duration_secs;
                tokio::spawn(async move {
                    if let Err(e) =
                        execute_fills(storage, event_bus, stream_bus, intent.id, duration).await
                    {
                        tracing::error!(intent_id = %intent.id, error = %e, "execution_failed");
                    }
                });
            }
        }

        Ok(())
    }
}

async fn execute_fills(
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    stream_bus: Arc<StreamBus>,
    intent_id: Uuid,
    execution_duration_secs: u64,
) -> Result<(), redis::RedisError> {
    let fills = storage.get_fills(&intent_id).await;
    if fills.is_empty() {
        tracing::warn!(intent_id = %intent_id, "No fills to execute");
        return Ok(());
    }

    let intent = match storage.get_intent(&intent_id).await {
        Some(i) => i,
        None => {
            tracing::error!(intent_id = %intent_id, "Intent not found during execution");
            return Ok(());
        }
    };

    let buyer_account_id = resolve_account(storage.pool(), &intent.user_id).await;

    tracing::info!(intent_id = %intent_id, fill_count = fills.len(), "executing_fills");

    if let Some(mut i) = storage.get_intent(&intent_id).await {
        i.status = IntentStatus::Executing;
        let _ = storage.update_intent(&i).await;
    }

    for fill in &fills {
        let exec_start = std::time::Instant::now();
        let tx_hash = format!("0x{}", Uuid::new_v4().simple());

        tracing::info!(
            intent_id = %intent_id, fill_id = %fill.id,
            solver_id = %fill.solver_id, filled_qty = fill.filled_qty,
            tx_hash = %tx_hash, "fill_execution_started"
        );

        let mut execution = Execution::new(intent_id, fill.solver_id.clone(), tx_hash);
        execution.status = ExecutionStatus::Executing;
        let _ = storage.insert_execution(&execution).await;

        event_bus.lock().await
            .publish(&Event::ExecutionStarted(execution.clone()))
            .await?;

        tokio::time::sleep(tokio::time::Duration::from_secs(execution_duration_secs)).await;

        execution.status = ExecutionStatus::Completed;
        let _ = storage.update_execution(&execution).await;

        let execution_id = execution.id;
        event_bus.lock().await
            .publish(&Event::ExecutionCompleted(execution))
            .await?;

        // Publish settlement trigger to Redis Streams
        let settlement_event = ExecutionCompletedEvent {
            execution_id,
            fill_id: fill.id,
            intent_id,
            solver_id: fill.solver_id.clone(),
            buyer_account_id: buyer_account_id.unwrap_or(Uuid::nil()),
            seller_account_id: Uuid::nil(),
            token_in: intent.token_in.clone(),
            token_out: intent.token_out.clone(),
            fee_rate: 0.001,
        };
        let _ = stream_bus.publish(STREAM_EXECUTION_COMPLETED, &settlement_event).await;

        let duration_ms = exec_start.elapsed().as_secs_f64() * 1000.0;
        counters::TRADES_TOTAL.inc();
        counters::TRADES_PER_SECOND.inc();
        histograms::TRADE_EXECUTION_DURATION.observe(exec_start.elapsed().as_secs_f64());

        tracing::info!(
            intent_id = %intent_id, execution_id = %execution_id,
            fill_id = %fill.id, duration_ms, "fill_executed_settlement_triggered"
        );
    }

    Ok(())
}

async fn resolve_account(pool: &sqlx::PgPool, user_id: &str) -> Option<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT a.id FROM accounts a JOIN users u ON u.id = a.user_id WHERE u.id::text = $1 LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}
