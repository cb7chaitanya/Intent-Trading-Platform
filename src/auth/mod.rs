pub mod handler;
pub mod jwt;
pub mod middleware;

use std::sync::Arc;

use axum::routing::post;
use axum::Router;

use crate::rbac::service::RbacService;
use crate::users::service::UserService;

use self::handler::AuthState;

/// Public auth routes (no JWT required).
pub fn public_router(user_service: Arc<UserService>, rbac_service: Arc<RbacService>) -> Router {
    let state = AuthState {
        user_service,
        rbac_service,
    };
    Router::new()
        .route("/auth/register", post(handler::register))
        .route("/auth/login", post(handler::login))
        .with_state(state)
}
