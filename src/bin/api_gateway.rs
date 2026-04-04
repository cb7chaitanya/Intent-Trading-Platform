use std::sync::Arc;

use intent_trading::api_key_service::ApiKeyService;
use intent_trading::config;
use intent_trading::gateway::router::{build_router, GatewayConfig};

#[tokio::main]
async fn main() {
    let cfg = config::init();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("api_gateway={},tower_http=info", cfg.log_level).into()),
        )
        .init();

    intent_trading::metrics::init();

    tracing::info!("Starting API Gateway [{}]", cfg.environment);
    tracing::info!("Listening on {}", cfg.gateway_addr);
    tracing::info!("Upstream: {}", cfg.upstream_url);

    // Connect to DB for API key validation
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&cfg.database_url)
        .await
        .expect("Failed to connect to PostgreSQL for gateway");

    let api_key_service = Arc::new(ApiKeyService::new(pg_pool));

    let gateway_config = GatewayConfig {
        upstream_url: cfg.upstream_url.clone(),
        redis_url: cfg.redis_url.clone(),
        database_url: cfg.database_url.clone(),
    };

    let app = build_router(gateway_config, api_key_service);

    let listener = tokio::net::TcpListener::bind(&cfg.gateway_addr)
        .await
        .expect("Failed to bind gateway address");

    axum::serve(listener, app).await.expect("Gateway error");
}
