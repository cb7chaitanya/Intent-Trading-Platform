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

    if let Some(mut intent) = storage.get_intent(&intent_id).await {
        intent.status = IntentStatus::Bidding;
        let _ = storage.update_intent(&intent).await;
        event_bus
            .lock()
            .await
            .publish(&Event::IntentBidding(intent))
            .await?;
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(auction_duration_secs)).await;

    let bid_count = storage.get_bids(&intent_id).await.len() as i64;
    gauges::BIDS_PER_AUCTION.set(bid_count);

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
    let Some(mut intent) = storage.get_intent(intent_id).await else {
        tracing::warn!(intent_id = %intent_id, "Intent not found during auction close");
        return Ok(());
    };

    let bids = sort_bids_by_price(storage, intent_id).await;
    if bids.is_empty() {
        tracing::warn!(intent_id = %intent_id, "auction_no_bids");
        intent.status = IntentStatus::Failed;
        let _ = storage.update_intent(&intent).await;
        event_bus
            .lock()
            .await
            .publish(&Event::IntentFailed(intent))
            .await?;
        return Ok(());
    }

    // Partial fill algorithm
    let fills = generate_partial_fills(&intent, &bids);
    let total_filled: i64 = fills.iter().map(|f| f.filled_qty).sum();
    let fully_filled = total_filled >= intent.amount_in;

    tracing::info!(
        intent_id = %intent_id,
        total_filled = total_filled,
        intent_amount = intent.amount_in,
        fill_count = fills.len(),
        fully_filled = fully_filled,
        "auction_fills_generated"
    );

    for fill in &fills {
        let _ = storage.insert_fill(fill).await;
        counters::SOLVER_WINS
            .with_label_values(&[&fill.solver_id])
            .inc();
    }

    if fully_filled {
        intent.status = IntentStatus::Matched;
    } else {
        intent.status = IntentStatus::PartiallyFilled;
    }
    let _ = storage.update_intent(&intent).await;

    // Publish match event with the best bid
    if let Some(best_fill) = fills.first() {
        if let Some(bid) = bids.iter().find(|b| b.solver_id == best_fill.solver_id).cloned() {
            event_bus
                .lock()
                .await
                .publish(&Event::IntentMatched {
                    intent: intent.clone(),
                    bid,
                })
                .await?;
        }
    }

    Ok(())
}

/// Sort bids by net value (amount_out - fee) descending, earliest timestamp breaks ties.
async fn sort_bids_by_price(storage: &Storage, intent_id: &Uuid) -> Vec<SolverBid> {
    let mut bids = storage.get_bids(intent_id).await;
    bids.sort_by(|a, b| {
        let net_a = a.amount_out.saturating_sub(a.fee);
        let net_b = b.amount_out.saturating_sub(b.fee);
        net_b.cmp(&net_a).then(a.timestamp.cmp(&b.timestamp))
    });
    bids
}

/// Generate partial fills from best to worst bid until intent quantity is filled.
fn generate_partial_fills(intent: &Intent, sorted_bids: &[SolverBid]) -> Vec<Fill> {
    let mut remaining = intent.amount_in;
    let mut fills = Vec::new();

    for bid in sorted_bids {
        if remaining <= 0 {
            break;
        }

        let net_value = bid.amount_out.saturating_sub(bid.fee);
        let fill_qty = remaining.min(net_value);

        if fill_qty <= 0 {
            continue;
        }

        fills.push(Fill::new(
            intent.id,
            bid.solver_id.clone(),
            bid.amount_out,
            net_value,
            fill_qty,
        ));

        remaining -= fill_qty;
    }

    fills
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_intent(amount_in: i64) -> Intent {
        Intent {
            id: Uuid::new_v4(),
            user_id: "user1".into(),
            token_in: "ETH".into(),
            token_out: "USDC".into(),
            amount_in,
            min_amount_out: 0,
            deadline: 9999999999,
            status: IntentStatus::Bidding,
            created_at: 1,
        }
    }

    fn make_bid(solver: &str, amount_out: i64, fee: i64, ts: i64) -> SolverBid {
        SolverBid {
            id: Uuid::new_v4(),
            intent_id: Uuid::new_v4(),
            solver_id: solver.into(),
            amount_out,
            fee,
            timestamp: ts,
        }
    }

    #[test]
    fn single_bid_full_fill() {
        let intent = make_intent(1000);
        let bids = vec![make_bid("s1", 1200, 50, 1)];

        let fills = generate_partial_fills(&intent, &bids);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].filled_qty, 1000);
    }

    #[test]
    fn multiple_bids_partial_fills() {
        let intent = make_intent(1000);
        let bids = vec![
            make_bid("s1", 500, 10, 1),  // net 490
            make_bid("s2", 600, 20, 2),  // net 580
        ];

        let fills = generate_partial_fills(&intent, &bids);
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].filled_qty, 490);
        assert_eq!(fills[1].filled_qty, 510);

        let total: i64 = fills.iter().map(|f| f.filled_qty).sum();
        assert_eq!(total, 1000);
    }

    #[test]
    fn insufficient_bids_partial_result() {
        let intent = make_intent(1000);
        let bids = vec![
            make_bid("s1", 400, 10, 1), // net 390
            make_bid("s2", 300, 10, 2), // net 290
        ];

        let fills = generate_partial_fills(&intent, &bids);
        let total: i64 = fills.iter().map(|f| f.filled_qty).sum();
        assert_eq!(total, 680);
        assert!(total < intent.amount_in);
    }

    #[test]
    fn no_bids_no_fills() {
        let intent = make_intent(1000);
        let fills = generate_partial_fills(&intent, &[]);
        assert!(fills.is_empty());
    }

    #[test]
    fn exact_fill_from_two_bids() {
        let intent = make_intent(500);
        let bids = vec![
            make_bid("s1", 300, 0, 1),
            make_bid("s2", 200, 0, 2),
        ];

        let fills = generate_partial_fills(&intent, &bids);
        let total: i64 = fills.iter().map(|f| f.filled_qty).sum();
        assert_eq!(total, 500);
        assert_eq!(fills.len(), 2);
    }

    #[test]
    fn stops_after_full_fill() {
        let intent = make_intent(100);
        let bids = vec![
            make_bid("s1", 200, 0, 1),
            make_bid("s2", 200, 0, 2),
            make_bid("s3", 200, 0, 3),
        ];

        let fills = generate_partial_fills(&intent, &bids);
        assert_eq!(fills.len(), 1); // only s1 needed
        assert_eq!(fills[0].filled_qty, 100);
    }
}
