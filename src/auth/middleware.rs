use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::jwt::{self, Claims};

const BEARER_PREFIX: &str = "Bearer ";

/// Axum middleware: extracts and validates JWT from the Authorization header.
/// On success, injects Claims into request extensions.
pub async fn require_auth(mut request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(BEARER_PREFIX));

    let Some(token) = token else {
        return (
            StatusCode::UNAUTHORIZED,
            "Missing Authorization header",
        )
            .into_response();
    };

    match jwt::validate_token(token).await {
        Ok(claims) => {
            tracing::debug!(
                user_id = %claims.sub,
                email = %claims.email,
                "JWT validated"
            );
            request.extensions_mut().insert(claims);
            next.run(request).await
        }
        Err(jwt::JwtError::Expired) => {
            (StatusCode::UNAUTHORIZED, "Token expired").into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "JWT validation failed");
            (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
        }
    }
}

/// Extract the authenticated user's claims from request extensions.
/// Use in handlers after the require_auth middleware.
pub fn extract_claims(request: &Request) -> Option<&Claims> {
    request.extensions().get::<Claims>()
}
