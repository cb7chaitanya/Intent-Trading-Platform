pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::service::MarketService;

pub fn router(market_service: Arc<MarketService>) -> Router {
    Router::new()
        .route("/markets", post(handler::create_market))
        .route("/markets", get(handler::list_markets))
        .route("/markets/{market_id}", get(handler::get_market))
        .with_state(market_service)
}
