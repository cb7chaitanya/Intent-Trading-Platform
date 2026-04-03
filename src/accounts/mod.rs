pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::service::AccountService;

pub fn router(account_service: Arc<AccountService>) -> Router {
    Router::new()
        .route("/accounts", post(handler::create_account))
        .route("/accounts/{user_id}", get(handler::get_accounts))
        .with_state(account_service)
}
