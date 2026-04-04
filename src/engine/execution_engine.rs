use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus, INTENT_MATCHED};
use crate::db::storage::Storage;
use crate::metrics::{counters, histograms};
use crate::models::execution::{Execution, ExecutionStatus};
use crate::models::intent::IntentStatus;

const EXECUTION_DURATION_SECS: u64 = 3;

pub struct ExecutionEngine {
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
}

impl ExecutionEngine {
    pub fn new(storage: Arc<Storage>, event_bus: EventBus) -> Self {
        Self {
            storage,
            event_bus: Arc::new(Mutex::new(event_bus)),
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
                    eprintln!("Failed to read message payload: {e}");
                    continue;
                }
            };

            let event = match serde_json::from_str::<Event>(&payload) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Failed to deserialize event: {e}");
                    continue;
                }
            };

            if let Event::IntentMatched { intent, bid } = event {
                let storage = Arc::clone(&self.storage);
                let event_bus = Arc::clone(&self.event_bus);
                tokio::spawn(async move {
                    if let Err(e) =
                        execute_intent(storage, event_bus, intent.id, bid.solver_id).await
                    {
                        eprintln!("Execution failed for intent {}: {e}", intent.id);
                    }
                });
            }
        }

        Ok(())
    }
}

async fn execute_intent(
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    intent_id: Uuid,
    solver_id: String,
) -> Result<(), redis::RedisError> {
    let exec_start = std::time::Instant::now();
    let tx_hash = format!("0x{}", Uuid::new_v4().simple());

    // Create execution record in Pending state
    let mut execution = Execution::new(intent_id, solver_id, tx_hash);

    // Transition to Executing
    execution.status = ExecutionStatus::Executing;
    storage.insert_execution(execution.clone());

    // Update intent status
    if let Some(mut intent) = storage.get_intent(&intent_id) {
        intent.status = IntentStatus::Executing;
        storage.update_intent(intent);
    }

    event_bus
        .lock()
        .await
        .publish(&Event::ExecutionStarted(execution.clone()))
        .await?;

    // Simulate execution time
    tokio::time::sleep(tokio::time::Duration::from_secs(EXECUTION_DURATION_SECS)).await;

    // Mark completed
    execution.status = ExecutionStatus::Completed;
    storage.update_execution(execution.clone());

    if let Some(mut intent) = storage.get_intent(&intent_id) {
        intent.status = IntentStatus::Completed;
        storage.update_intent(intent);
    }

    event_bus
        .lock()
        .await
        .publish(&Event::ExecutionCompleted(execution))
        .await?;

    counters::TRADES_TOTAL.inc();
    counters::TRADES_PER_SECOND.inc();
    histograms::TRADE_EXECUTION_DURATION.observe(exec_start.elapsed().as_secs_f64());

    Ok(())
}
