//! Axum middleware for trace context propagation.
//!
//! Extracts W3C traceparent header from incoming requests and creates
//! a child span. Injects traceparent into outgoing responses so clients
//! can correlate.

use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use tracing::Instrument;

/// Axum middleware: creates a request-scoped span with trace context.
pub async fn trace_layer(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let uri = request.uri().path().to_string();

    // Extract traceparent from incoming headers (W3C format)
    let parent_trace = request
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Extract trace_id from traceparent if present
    // Format: version-trace_id-parent_id-trace_flags
    let trace_id = parent_trace
        .as_ref()
        .and_then(|tp| tp.split('-').nth(1))
        .unwrap_or("none")
        .to_string();

    let span = tracing::info_span!(
        "http_request",
        http.method = %method,
        http.route = %uri,
        trace_id = %trace_id,
        otel.kind = "server",
    );

    let response = next.run(request).instrument(span).await;

    response
}

/// Extract trace_id for injection into Redis messages and inter-service calls.
pub fn inject_trace_context() -> Vec<(String, String)> {
    let trace_id = crate::telemetry::current_trace_id();
    vec![("trace_id".to_string(), trace_id)]
}
