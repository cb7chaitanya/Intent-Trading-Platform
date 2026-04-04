use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus, INTENT_CREATED};
use crate::db::storage::Storage;
use crate::metrics::{counters, gauges, histograms};
use crate::models::bid::SolverBid;
use crate::models::fill::Fill;
use crate::models::intent::{Intent, IntentStatus};

pub struct AuctionEngine {
    storage: Arc<Storage>,
    event_bus: Arc<Mutex<EventBus>>,
    auction_duration_secs: u64,
}

impl AuctionEngine {
    pub fn new(storage: Arc<Storage>, event_bus: EventBus, auction_duration_secs: u64) -> Self {
        Self {
            storage,
            event_bus: Arc::new(Mutex::new(event_bus)),
            auction_duration_secs,
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
                    tracing::warn!(error = %e, "Failed to read auction message payload");
                    continue;
                }
            };

            let event = match serde_json::from_str::<Event>(&payload) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to deserialize auction event");
                    continue;
                }
            };

            if let Event::IntentCreated(intent) = event {
                let storage = Arc::clone(&self.storage);
                let event_bus = Arc::clone(&self.event_bus);
                let duration = self.auction_duration_secs;
                tokio::spawn(async move {
                    tracing::info!(intent_id = %intent.id, "auction_started");
                    if let Err(e) = run_auction(storage, event_bus, intent.id, duration).await {
                        tracing::error!(intent_id = %intent.id, error = %e, "auction_failed");
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
    auction_duration_secs: u64,
) -> Result<(), redis::RedisError> {
    gauges::ACTIVE_AUCTIONS.inc();
    let auction_start = std::time::Instant::now();

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
    tokio::time::sleep(tokio::time::Duration::from_secs(auction_duration_secs)).await;

    // Record bid count for this auction
    let bid_count = storage.get_bids(&intent_id).len() as i64;
    gauges::BIDS_PER_AUCTION.set(bid_count);

    // Close auction with matching latency tracking
    let match_start = std::time::Instant::now();
    let result = close_auction(&storage, &event_bus, &intent_id).await;
    histograms::MATCHING_ENGINE_LATENCY.observe(match_start.elapsed().as_secs_f64());

    histograms::AUCTION_DURATION.observe(auction_start.elapsed().as_secs_f64());
    counters::AUCTIONS_TOTAL.inc();
    gauges::ACTIVE_AUCTIONS.dec();

    result
}

async fn close_auction(
    storage: &Storage,
    event_bus: &Arc<Mutex<EventBus>>,
    intent_id: &Uuid,
) -> Result<(), redis::RedisError> {
    let Some(mut intent) = storage.get_intent(intent_id) else {
        tracing::warn!(intent_id = %intent_id, "Intent not found during auction close");
        return Ok(());
    };

    match select_best_bid(storage, intent_id) {
        Some(bid) => {
            tracing::info!(
                intent_id = %intent_id,
                solver_id = %bid.solver_id,
                amount_out = bid.amount_out,
                "auction_matched"
            );

            let fill = generate_fill(&intent, &bid);
            storage.insert_fill(fill);

            intent.status = IntentStatus::Matched;
            storage.update_intent(intent.clone());

            counters::SOLVER_WINS
                .with_label_values(&[&bid.solver_id])
                .inc();

            event_bus
                .lock()
                .await
                .publish(&Event::IntentMatched { intent, bid })
                .await?;
        }
        None => {
            tracing::warn!(intent_id = %intent_id, "auction_no_bids");

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
        String::new(),
    )
}
