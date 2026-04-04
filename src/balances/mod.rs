pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::rbac::middleware::require_perm;

use self::service::BalanceService;

pub fn router(balance_service: Arc<BalanceService>) -> Router {
    let write = Router::new()
        .route("/balances/deposit", post(handler::deposit))
        .route_layer(middleware::from_fn(require_perm("balance:deposit")))
        .route("/balances/withdraw", post(handler::withdraw))
        .route_layer(middleware::from_fn(require_perm("balance:withdraw")));

    let read = Router::new()
        .route("/balances/{account_id}", get(handler::get_balances))
        .route_layer(middleware::from_fn(require_perm("balance:read")));

    write.merge(read).with_state(balance_service)
}
