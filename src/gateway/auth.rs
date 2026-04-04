use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api_key_service::ApiKeyService;
use crate::jwt;

const API_KEY_HEADER: &str = "x-api-key";
const BEARER_PREFIX: &str = "Bearer ";

/// Gateway auth: accepts JWT Bearer token or database-backed API key.
/// On success, injects x-user-id and x-user-email/x-user-permissions headers.
pub async fn validate_auth(request: Request, next: Next) -> Response {
    // Try JWT first
    if let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(BEARER_PREFIX))
    {
        return match jwt::validate_token(token) {
            Ok(claims) => {
                let mut request = request;
                let headers = request.headers_mut();
                let _ = headers.insert("x-user-id", claims.sub.to_string().parse().unwrap());
                let _ = headers.insert("x-user-email", claims.email.parse().unwrap());
                next.run(request).await
            }
            Err(jwt::JwtError::Expired) => {
                (StatusCode::UNAUTHORIZED, "Token expired").into_response()
            }
            Err(e) => {
                tracing::warn!(error = %e, "Gateway JWT validation failed");
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
        };
    }

    // Fallback to API key (database-backed)
    if let Some(raw_key) = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        let svc = request.extensions().get::<Arc<ApiKeyService>>().cloned();
        let Some(svc) = svc else {
            // No ApiKeyService available — reject
            tracing::warn!("ApiKeyService not available for API key validation");
            return (StatusCode::INTERNAL_SERVER_ERROR, "API key validation unavailable")
                .into_response();
        };

        return match svc.validate_key(raw_key).await {
            Ok(key) => {
                let mut request = request;
                let headers = request.headers_mut();
                let _ = headers.insert("x-user-id", key.user_id.to_string().parse().unwrap());
                let perms = key.permissions.join(",");
                let _ = headers.insert("x-user-permissions", perms.parse().unwrap());
                next.run(request).await
            }
            Err(crate::api_key_service::ApiKeyError::Revoked) => {
                (StatusCode::FORBIDDEN, "API key revoked").into_response()
            }
            Err(e) => {
                tracing::warn!(error = %e, "API key validation failed");
                (StatusCode::UNAUTHORIZED, "Invalid API key").into_response()
            }
        };
    }

    tracing::warn!("no auth credentials provided");
    (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
}
