pub mod bids;
pub mod intents;
pub mod orderbook;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tokio::sync::Mutex;

use crate::services::bid_service::BidService;
use crate::services::intent_service::IntentService;

#[derive(Clone)]
pub struct AppState {
    pub intent_service: Arc<Mutex<IntentService>>,
    pub bid_service: Arc<Mutex<BidService>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/intents", post(intents::create_intent))
        .route("/intents", get(intents::list_intents))
        .route("/intents/{id}", get(intents::get_intent))
        .route("/bids", post(bids::submit_bid))
        .route("/orderbook/{intent_id}", get(orderbook::get_orderbook))
        .with_state(state)
}
