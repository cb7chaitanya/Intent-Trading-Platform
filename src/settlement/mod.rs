pub mod engine;
pub mod handler;
pub mod model;
pub mod retry;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::engine::SettlementEngine;

pub fn router(settlement_engine: Arc<SettlementEngine>) -> Router {
    Router::new()
        .route("/trades", post(handler::create_trade))
        .route("/trades/{trade_id}/settle", post(handler::settle_trade))
        .route("/trades/{trade_id}", get(handler::get_trade))
        .with_state(settlement_engine)
}
