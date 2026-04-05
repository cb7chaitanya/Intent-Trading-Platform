pub mod handler;
pub mod model;
pub mod repository;
pub mod service;
pub mod stats;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use self::service::SolverReputationService;

/// Public routes — leaderboard + registration (no auth).
pub fn router(solver_service: Arc<SolverReputationService>) -> Router {
    Router::new()
        .route("/solvers/register", post(handler::register_solver))
        .route("/solvers/top", get(handler::get_top_solvers))
        .route("/solvers/leaderboard", get(handler::get_leaderboard))
        .route("/solvers/{id}", get(handler::get_solver))
        .route("/solvers/{id}/dashboard", get(handler::get_dashboard))
        .route("/solvers/{id}/stats", get(handler::get_solver_stats))
        .with_state(solver_service)
}

/// Protected routes — profile management (requires auth).
pub fn protected_router(solver_service: Arc<SolverReputationService>) -> Router {
    Router::new()
        .route("/solvers/{id}", put(handler::update_solver))
        .route(
            "/solvers/{id}/deactivate",
            delete(handler::deactivate_solver),
        )
        .with_state(solver_service)
}
