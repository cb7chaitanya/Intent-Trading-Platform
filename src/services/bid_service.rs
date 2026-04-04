use std::sync::Arc;

use uuid::Uuid;

use crate::db::redis::{Event, EventBus};
use crate::db::storage::Storage;
use crate::db::stream_bus::{StreamBus, STREAM_BID_SUBMITTED};
use crate::metrics::counters;
use crate::models::bid::SolverBid;

pub struct BidService {
    storage: Arc<Storage>,
    event_bus: EventBus,
    stream_bus: Arc<StreamBus>,
}

impl BidService {
    pub fn new(storage: Arc<Storage>, event_bus: EventBus, stream_bus: Arc<StreamBus>) -> Self {
        Self {
            storage,
            event_bus,
            stream_bus,
        }
    }

    pub async fn submit_bid(
        &mut self,
        intent_id: Uuid,
        solver_id: String,
        amount_out: u64,
        fee: u64,
    ) -> Result<SolverBid, redis::RedisError> {
        let bid = SolverBid::new(intent_id, solver_id, amount_out, fee);
        let _ = self.storage.insert_bid(&bid).await;
        self.event_bus
            .publish(&Event::BidSubmitted(bid.clone()))
            .await?;

        let _ = self.stream_bus.publish(STREAM_BID_SUBMITTED, &bid).await;

        counters::BIDS_TOTAL.inc();

        tracing::info!(
            bid_id = %bid.id,
            intent_id = %bid.intent_id,
            solver_id = %bid.solver_id,
            amount_out = bid.amount_out,
            fee = bid.fee,
            "bid_submitted"
        );

        Ok(bid)
    }

    pub async fn get_bids(&self, intent_id: &Uuid) -> Vec<SolverBid> {
        self.storage.get_bids(intent_id).await
    }

    pub async fn get_best_bid(&self, intent_id: &Uuid) -> Option<SolverBid> {
        let bids = self.storage.get_bids(intent_id).await;
        bids.into_iter().max_by(|a, b| {
            let net_a = a.amount_out.saturating_sub(a.fee);
            let net_b = b.amount_out.saturating_sub(b.fee);
            net_a.cmp(&net_b).then(b.timestamp.cmp(&a.timestamp))
        })
    }

    pub async fn build_orderbook(&self, intent_id: &Uuid) -> Vec<SolverBid> {
        let mut bids = self.storage.get_bids(intent_id).await;
        bids.sort_by(|a, b| {
            let net_a = a.amount_out.saturating_sub(a.fee);
            let net_b = b.amount_out.saturating_sub(b.fee);
            net_b.cmp(&net_a).then(a.timestamp.cmp(&b.timestamp))
        });
        bids
    }
}
