use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

const API_KEY_HEADER: &str = "x-api-key";

pub async fn validate_api_key(request: Request, next: Next) -> Response {
    let key = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok());

    match key {
        Some(k) if is_valid_key(k) => next.run(request).await,
        Some(_) => {
            tracing::warn!("invalid API key presented");
            (axum::http::StatusCode::FORBIDDEN, "Forbidden").into_response()
        }
        None => {
            tracing::warn!("missing API key header");
            (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
        }
    }
}

fn is_valid_key(key: &str) -> bool {
    key.starts_with("itx_") && key.len() >= 8
}
