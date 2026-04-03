pub mod feed;
pub mod handler;
pub mod server;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use self::feed::WsFeed;

pub fn router(feed: Arc<WsFeed>) -> Router {
    Router::new()
        .route("/ws/feed", get(handler::ws_handler))
        .with_state(feed)
}
