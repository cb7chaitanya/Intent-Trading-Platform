pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use self::service::SolverReputationService;

pub fn router(solver_service: Arc<SolverReputationService>) -> Router {
    Router::new()
        .route("/solvers/top", get(handler::get_top_solvers))
        .route("/solvers/{id}", get(handler::get_solver))
        .with_state(solver_service)
}
