pub mod handler;
pub mod model;
pub mod repository;
pub mod service;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::service::UserService;

pub fn router(user_service: Arc<UserService>) -> Router {
    Router::new()
        .route("/users/register", post(handler::register))
        .route("/users/login", post(handler::login))
        .route("/users/{id}", get(handler::get_user))
        .with_state(user_service)
}
