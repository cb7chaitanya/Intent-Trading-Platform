use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::jwt;

const API_KEY_HEADER: &str = "x-api-key";
const BEARER_PREFIX: &str = "Bearer ";

/// Gateway auth: accepts JWT Bearer token or API key.
/// On JWT success, injects x-user-id and x-user-email headers for upstream.
pub async fn validate_auth(mut request: Request, next: Next) -> Response {
    // Try JWT first
    if let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(BEARER_PREFIX))
    {
        return match jwt::validate_token(token) {
            Ok(claims) => {
                let headers = request.headers_mut();
                let _ = headers.insert("x-user-id", claims.sub.to_string().parse().unwrap());
                let _ = headers.insert("x-user-email", claims.email.parse().unwrap());
                next.run(request).await
            }
            Err(jwt::JwtError::Expired) => {
                (axum::http::StatusCode::UNAUTHORIZED, "Token expired").into_response()
            }
            Err(e) => {
                tracing::warn!(error = %e, "Gateway JWT validation failed");
                (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
        };
    }

    // Fallback to API key
    if let Some(key) = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        if is_valid_api_key(key) {
            return next.run(request).await;
        }
        tracing::warn!("invalid API key presented");
        return (axum::http::StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    tracing::warn!("no auth credentials provided");
    (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
}

fn is_valid_api_key(key: &str) -> bool {
    key.starts_with("itx_") && key.len() >= 8
}
