pub mod counters;
pub mod gauges;
pub mod histograms;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use once_cell::sync::Lazy;
use prometheus::{Encoder, Registry, TextEncoder};

pub static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

/// Handler for GET /metrics
async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();

    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                encoder.format_type().to_string(),
            )],
            buffer,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {e}"),
        )
            .into_response(),
    }
}

/// Returns a Router with GET /metrics
pub fn router() -> Router {
    Router::new().route("/metrics", get(metrics_handler))
}

/// Force-initialize all lazy metrics so they appear in /metrics even before
/// first use. Call this once at startup.
pub fn init() {
    // Touch each lazy to trigger registration
    let _ = &*counters::INTENTS_TOTAL;
    let _ = &*counters::TRADES_TOTAL;
    let _ = &*counters::BIDS_TOTAL;
    let _ = &*counters::SETTLEMENT_SUCCESS_TOTAL;
    let _ = &*counters::SETTLEMENT_FAILURES_TOTAL;
    let _ = &*counters::API_REQUESTS_TOTAL;
    let _ = &*counters::FEES_COLLECTED_TOTAL;
    let _ = &*counters::TRADE_VOLUME;
    let _ = &*counters::SOLVER_WINS;
    let _ = &*counters::TRADES_PER_SECOND;
    let _ = &*gauges::ACTIVE_AUCTIONS;
    let _ = &*gauges::WEBSOCKET_CONNECTIONS;
    let _ = &*gauges::BIDS_PER_AUCTION;
    let _ = &*counters::AUCTIONS_TOTAL;
    let _ = &*counters::DB_QUERIES_TOTAL;
    let _ = &*histograms::API_REQUEST_DURATION;
    let _ = &*histograms::MATCHING_ENGINE_LATENCY;
    let _ = &*histograms::AUCTION_DURATION;
    let _ = &*histograms::TRADE_EXECUTION_DURATION;
    let _ = &*histograms::DB_QUERY_DURATION;
    let _ = &*histograms::SETTLEMENT_DURATION;
}
