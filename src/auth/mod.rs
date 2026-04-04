pub mod handler;
pub mod jwt;
pub mod middleware;

use std::sync::Arc;

use axum::routing::post;
use axum::Router;

use crate::users::service::UserService;

/// Public auth routes (no JWT required).
pub fn public_router(user_service: Arc<UserService>) -> Router {
    Router::new()
        .route("/auth/register", post(handler::register))
        .route("/auth/login", post(handler::login))
        .with_state(user_service)
}
