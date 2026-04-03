use axum::middleware;
use axum::routing::any;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use super::auth::validate_api_key;
use super::proxy::{proxy_handler, ProxyState};
use super::rate_limit::{RateLimitLayer, RateLimiter};

pub struct GatewayConfig {
    pub upstream_url: String,
    pub redis_url: String,
}

pub fn build_router(config: GatewayConfig) -> Router {
    let proxy_state = ProxyState {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client"),
        upstream_url: config.upstream_url,
    };

    let rate_limiter = RateLimiter::new();

    let service_routes = Router::new()
        .route("/users/{*rest}", any(proxy_handler))
        .route("/accounts/{*rest}", any(proxy_handler))
        .route("/balances/{*rest}", any(proxy_handler))
        .route("/intents/{*rest}", any(proxy_handler))
        .route("/intents", any(proxy_handler))
        .route("/markets/{*rest}", any(proxy_handler))
        .route("/markets", any(proxy_handler))
        .route("/trades/{*rest}", any(proxy_handler))
        .route("/trades", any(proxy_handler))
        .route("/solvers/{*rest}", any(proxy_handler))
        .route("/bids", any(proxy_handler))
        .route("/orderbook/{*rest}", any(proxy_handler))
        .route("/candles/{*rest}", any(proxy_handler))
        .route("/market-data/{*rest}", any(proxy_handler))
        .route("/ledger/{*rest}", any(proxy_handler))
        .route("/settlement/{*rest}", any(proxy_handler))
        .with_state(proxy_state)
        .layer(middleware::from_fn(validate_api_key))
        .layer(RateLimitLayer::new(rate_limiter));

    let health = Router::new().route("/health", any(|| async { "ok" }));

    Router::new()
        .merge(health)
        .merge(service_routes)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive()),
        )
}
