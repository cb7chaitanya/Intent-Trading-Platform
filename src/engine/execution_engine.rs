use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus, INTENT_MATCHED};
use crate::db::storage::Storage;
use crate::metrics::{counters, histograms};
use crate::models::execution::{Execution, ExecutionStatus};
use crate::models::intent::IntentStatus;

pub struct ExecutionEngine {
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    execution_duration_secs: u64,
}

impl ExecutionEngine {
    pub fn new(storage: Arc<Storage>, event_bus: EventBus, execution_duration_secs: u64) -> Self {
        Self {
            storage,
            event_bus: Arc::new(Mutex::new(event_bus)),
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
                let duration = self.execution_duration_secs;
                tokio::spawn(async move {
                    if let Err(e) = execute_fills(storage, event_bus, intent.id, duration).await {
                        tracing::error!(intent_id = %intent.id, error = %e, "execution_failed");
                    }
                });
            }
        }

        Ok(())
    }
}

/// Execute all fills for an intent. Each fill gets its own execution record.
async fn execute_fills(
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    intent_id: Uuid,
    execution_duration_secs: u64,
) -> Result<(), redis::RedisError> {
    let fills = storage.get_fills(&intent_id).await;
    if fills.is_empty() {
        tracing::warn!(intent_id = %intent_id, "No fills to execute");
        return Ok(());
    }

    tracing::info!(
        intent_id = %intent_id,
        fill_count = fills.len(),
        "executing_fills"
    );

    // Mark intent as executing
    if let Some(mut intent) = storage.get_intent(&intent_id).await {
        intent.status = IntentStatus::Executing;
        let _ = storage.update_intent(&intent).await;
    }

    // Execute each fill
    for fill in &fills {
        let exec_start = std::time::Instant::now();
        let tx_hash = format!("0x{}", Uuid::new_v4().simple());

        tracing::info!(
            intent_id = %intent_id,
            fill_id = %fill.id,
            solver_id = %fill.solver_id,
            filled_qty = fill.filled_qty,
            tx_hash = %tx_hash,
            "fill_execution_started"
        );

        let mut execution = Execution::new(
            intent_id,
            fill.solver_id.clone(),
            tx_hash,
        );
        execution.status = ExecutionStatus::Executing;
        let _ = storage.insert_execution(&execution).await;

        event_bus
            .lock()
            .await
            .publish(&Event::ExecutionStarted(execution.clone()))
            .await?;

        // Simulate execution time
        tokio::time::sleep(tokio::time::Duration::from_secs(execution_duration_secs)).await;

        execution.status = ExecutionStatus::Completed;
        let _ = storage.update_execution(&execution).await;

        let execution_id = execution.id;
        event_bus
            .lock()
            .await
            .publish(&Event::ExecutionCompleted(execution))
            .await?;

        let duration_ms = exec_start.elapsed().as_secs_f64() * 1000.0;
        counters::TRADES_TOTAL.inc();
        counters::TRADES_PER_SECOND.inc();
        histograms::TRADE_EXECUTION_DURATION.observe(exec_start.elapsed().as_secs_f64());

        tracing::info!(
            intent_id = %intent_id,
            execution_id = %execution_id,
            fill_id = %fill.id,
            duration_ms = duration_ms,
            "fill_executed"
        );
    }

    // Mark intent as completed
    if let Some(mut intent) = storage.get_intent(&intent_id).await {
        intent.status = IntentStatus::Completed;
        let _ = storage.update_intent(&intent).await;
    }

    tracing::info!(
        intent_id = %intent_id,
        fills_executed = fills.len(),
        "intent_execution_completed"
    );

    Ok(())
}
