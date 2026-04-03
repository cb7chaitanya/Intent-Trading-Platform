pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use self::service::LedgerService;

pub fn router(ledger_service: Arc<LedgerService>) -> Router {
    Router::new()
        .route("/ledger/{account_id}", get(handler::get_entries))
        .route(
            "/ledger/reference/{reference_id}",
            get(handler::get_entries_by_reference),
        )
        .with_state(ledger_service)
}
