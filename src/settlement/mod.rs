pub mod engine;
pub mod handler;
pub mod model;
pub mod retry;
pub mod worker;

use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::rbac::middleware::require_perm;

use self::engine::SettlementEngine;

pub fn router(settlement_engine: Arc<SettlementEngine>) -> Router {
    let write = Router::new()
        .route("/trades", post(handler::create_trade))
        .route("/trades/{trade_id}/settle", post(handler::settle_trade))
        .route("/intents/{intent_id}/settle", post(handler::settle_intent))
        .route_layer(middleware::from_fn(require_perm("trade:settle")));

    let read = Router::new()
        .route("/trades/{trade_id}", get(handler::get_trade))
        .route_layer(middleware::from_fn(require_perm("trade:read")));

    write.merge(read).with_state(settlement_engine)
}
