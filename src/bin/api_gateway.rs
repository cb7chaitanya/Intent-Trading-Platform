use intent_trading::gateway::router::{build_router, GatewayConfig};

const GATEWAY_ADDR: &str = "0.0.0.0:4000";
const UPSTREAM_URL: &str = "http://127.0.0.1:3000";
const REDIS_URL: &str = "redis://127.0.0.1:6379";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "api_gateway=info,tower_http=info".into()),
        )
        .init();

    tracing::info!("Starting API Gateway on {GATEWAY_ADDR}");
    tracing::info!("Upstream: {UPSTREAM_URL}");

    let config = GatewayConfig {
        upstream_url: UPSTREAM_URL.to_string(),
        redis_url: REDIS_URL.to_string(),
    };

    let app = build_router(config);

    let listener = tokio::net::TcpListener::bind(GATEWAY_ADDR)
        .await
        .expect("Failed to bind gateway address");

    tracing::info!("Gateway listening on {GATEWAY_ADDR}");
    axum::serve(listener, app).await.expect("Gateway error");
}
