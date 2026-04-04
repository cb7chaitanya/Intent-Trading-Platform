use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::auth::jwt::Claims;

/// Create a middleware closure that checks if the user has the required role.
/// Must be applied after `require_auth` middleware (which injects Claims).
///
/// Usage:
///   .layer(middleware::from_fn(require_role("admin")))
pub fn require_role(
    role: &'static str,
) -> impl Fn(Request, axum::middleware::Next) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Response> + Send>,
> + Clone
       + Send {
    move |request: Request, next: axum::middleware::Next| {
        Box::pin(async move {
            let claims = match request.extensions().get::<Claims>() {
                Some(c) => c.clone(),
                None => {
                    return (StatusCode::UNAUTHORIZED, "Authentication required")
                        .into_response();
                }
            };

            // Check if the role is in the JWT permissions/roles
            let has_role = claims.permissions.iter().any(|p| p == role || p == "admin");

            if !has_role {
                tracing::warn!(
                    user_id = %claims.sub,
                    required_role = role,
                    user_roles = ?claims.permissions,
                    "access_denied"
                );
                return (
                    StatusCode::FORBIDDEN,
                    format!("Role '{role}' required"),
                )
                    .into_response();
            }

            next.run(request).await
        })
    }
}

/// Create a middleware closure that checks if the user has permission
/// for a specific resource and action.
pub fn require_permission(
    resource: &'static str,
    action: &'static str,
) -> impl Fn(Request, axum::middleware::Next) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Response> + Send>,
> + Clone
       + Send {
    move |request: Request, next: axum::middleware::Next| {
        Box::pin(async move {
            let claims = match request.extensions().get::<Claims>() {
                Some(c) => c.clone(),
                None => {
                    return (StatusCode::UNAUTHORIZED, "Authentication required")
                        .into_response();
                }
            };

            // admin has all permissions
            if claims.permissions.contains(&"admin".to_string()) {
                return next.run(request).await;
            }

            // Check for specific permission in format "resource:action"
            let required = format!("{resource}:{action}");
            let has_perm = claims.permissions.iter().any(|p| {
                p == &required || p == &format!("{resource}:*") || p == "*:*"
            });

            if !has_perm {
                tracing::warn!(
                    user_id = %claims.sub,
                    resource = resource,
                    action = action,
                    "permission_denied"
                );
                return (
                    StatusCode::FORBIDDEN,
                    format!("Permission '{resource}:{action}' required"),
                )
                    .into_response();
            }

            next.run(request).await
        })
    }
}
