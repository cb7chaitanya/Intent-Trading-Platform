pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::service::BalanceService;

pub fn router(balance_service: Arc<BalanceService>) -> Router {
    Router::new()
        .route("/balances/deposit", post(handler::deposit))
        .route("/balances/withdraw", post(handler::withdraw))
        .route("/balances/{account_id}", get(handler::get_balances))
        .with_state(balance_service)
}
