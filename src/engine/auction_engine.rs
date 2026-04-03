use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus, INTENT_CREATED};
use crate::db::storage::Storage;
use crate::models::bid::SolverBid;
use crate::models::fill::Fill;
use crate::models::intent::{Intent, IntentStatus};

const AUCTION_DURATION_SECS: u64 = 10;

pub struct AuctionEngine {
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
}

impl AuctionEngine {
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

        pubsub.subscribe(INTENT_CREATED).await?;

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

            if let Event::IntentCreated(intent) = event {
                let storage = Arc::clone(&self.storage);
                let event_bus = Arc::clone(&self.event_bus);
                tokio::spawn(async move {
                    if let Err(e) = run_auction(storage, event_bus, intent.id).await {
                        eprintln!("Auction failed for intent {}: {e}", intent.id);
                    }
                });
            }
        }

        Ok(())
    }
}

async fn run_auction(
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    intent_id: Uuid,
) -> Result<(), redis::RedisError> {
    // Start bidding phase
    if let Some(mut intent) = storage.get_intent(&intent_id) {
        intent.status = IntentStatus::Bidding;
        storage.update_intent(intent.clone());
        event_bus
            .lock()
            .await
            .publish(&Event::IntentBidding(intent))
            .await?;
    }

    // Wait for bids
    tokio::time::sleep(tokio::time::Duration::from_secs(AUCTION_DURATION_SECS)).await;

    // Close auction
    close_auction(&storage, &event_bus, &intent_id).await
}

async fn close_auction(
    storage: &Storage,
    event_bus: &Arc<Mutex<EventBus>>,
    intent_id: &Uuid,
) -> Result<(), redis::RedisError> {
    let Some(mut intent) = storage.get_intent(intent_id) else {
        eprintln!("Intent {intent_id} not found during auction close");
        return Ok(());
    };

    match select_best_bid(storage, intent_id) {
        Some(bid) => {
            let fill = generate_fill(&intent, &bid);
            storage.insert_fill(fill);

            intent.status = IntentStatus::Matched;
            storage.update_intent(intent.clone());

            event_bus
                .lock()
                .await
                .publish(&Event::IntentMatched { intent, bid })
                .await?;
        }
        None => {
            intent.status = IntentStatus::Failed;
            storage.update_intent(intent.clone());

            event_bus
                .lock()
                .await
                .publish(&Event::IntentFailed(intent))
                .await?;
        }
    }

    Ok(())
}

fn select_best_bid(storage: &Storage, intent_id: &Uuid) -> Option<SolverBid> {
    let bids = storage.get_bids(intent_id);
    bids.into_iter().max_by(|a, b| {
        let net_a = a.amount_out.saturating_sub(a.fee);
        let net_b = b.amount_out.saturating_sub(b.fee);
        net_a.cmp(&net_b).then(b.timestamp.cmp(&a.timestamp))
    })
}

fn generate_fill(intent: &Intent, bid: &SolverBid) -> Fill {
    Fill::new(
        intent.id,
        bid.solver_id.clone(),
        bid.amount_out,
        intent.amount_in,
        String::new(), // tx_hash populated during execution
    )
}
