pub mod bids;
pub mod intents;
pub mod orderbook;

use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::Mutex;

use crate::rbac::middleware::require_perm;
use crate::services::bid_service::BidService;
use crate::services::intent_service::IntentService;

#[derive(Clone)]
pub struct AppState {
    pub intent_service: Arc<Mutex<IntentService>>,
    pub bid_service: Arc<Mutex<BidService>>,
}

pub fn router(state: AppState) -> Router {
    // Intent write routes
    let intent_write = Router::new()
        .route("/intents", post(intents::create_intent))
        .route_layer(middleware::from_fn(require_perm("intent:create")));

    // Intent read routes
    let intent_read = Router::new()
        .route("/intents", get(intents::list_intents))
        .route("/intents/{id}", get(intents::get_intent))
        .route_layer(middleware::from_fn(require_perm("intent:read")));

    // Bid write routes
    let bid_write = Router::new()
        .route("/bids", post(bids::submit_bid))
        .route_layer(middleware::from_fn(require_perm("bid:create")));

    // Orderbook read routes
    let orderbook_read = Router::new()
        .route("/orderbook/{intent_id}", get(orderbook::get_orderbook))
        .route_layer(middleware::from_fn(require_perm("market:read")));

    intent_write
        .merge(intent_read)
        .merge(bid_write)
        .merge(orderbook_read)
        .with_state(state)
}
