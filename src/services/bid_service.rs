use std::sync::Arc;

use uuid::Uuid;

use crate::db::redis::{Event, EventBus};
use crate::db::storage::Storage;
use crate::db::stream_bus::{StreamBus, STREAM_BID_SUBMITTED};
use crate::metrics::counters;
use crate::models::bid::SolverBid;
use crate::risk::service::RiskEngine;

#[derive(Debug)]
pub enum BidError {
    RedisError(redis::RedisError),
    RiskRejected(String),
    IntentNotFound,
}

impl std::fmt::Display for BidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BidError::RedisError(e) => write!(f, "Redis error: {e}"),
            BidError::RiskRejected(e) => write!(f, "Bid rejected: {e}"),
            BidError::IntentNotFound => write!(f, "Intent not found"),
        }
    }
}

impl From<redis::RedisError> for BidError {
    fn from(e: redis::RedisError) -> Self {
        BidError::RedisError(e)
    }
}

pub struct BidService {
    storage: Arc<Storage>,
    event_bus: EventBus,
    stream_bus: Arc<StreamBus>,
    risk_engine: Arc<RiskEngine>,
}

impl BidService {
    pub fn new(
        storage: Arc<Storage>,
        event_bus: EventBus,
        stream_bus: Arc<StreamBus>,
        risk_engine: Arc<RiskEngine>,
    ) -> Self {
        Self { storage, event_bus, stream_bus, risk_engine }
    }

    pub async fn submit_bid(
        &mut self,
        intent_id: Uuid,
        solver_id: String,
        amount_out: u64,
        fee: u64,
    ) -> Result<SolverBid, BidError> {
        // Look up intent for risk validation context
        let intent = self.storage.get_intent(&intent_id).await
            .ok_or(BidError::IntentNotFound)?;

        // Validate bid against oracle + cross-market
        self.risk_engine
            .validate_bid(
                &intent.token_in,
                &intent.token_out,
                amount_out as i64,
                fee as i64,
                intent.amount_in,
            )
            .await
            .map_err(|e| BidError::RiskRejected(e.to_string()))?;

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
