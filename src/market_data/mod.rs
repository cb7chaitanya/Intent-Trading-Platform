pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use self::service::MarketDataService;

pub fn router(market_data_service: Arc<MarketDataService>) -> Router {
    Router::new()
        .route("/market-data/trades/{market_id}", get(handler::get_trades))
        .route("/orderbook/{market_id}", get(handler::get_orderbook))
        .route("/candles/{market_id}", get(handler::get_candles))
        .with_state(market_data_service)
}
