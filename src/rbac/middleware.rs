use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::auth::jwt::Claims;

/// Extract permissions from either:
/// 1. JWT Claims (injected by require_auth middleware)
/// 2. x-user-permissions header (injected by API gateway for API key users)
fn extract_permissions(request: &Request) -> Vec<String> {
    // Try JWT claims first
    if let Some(claims) = request.extensions().get::<Claims>() {
        return claims.permissions.clone();
    }

    // Fallback: x-user-permissions header (comma-separated)
    request
        .headers()
        .get("x-user-permissions")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default()
}

fn extract_user_id(request: &Request) -> String {
    if let Some(claims) = request.extensions().get::<Claims>() {
        return claims.sub.to_string();
    }
    request
        .headers()
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn has_permission(permissions: &[String], required: &str) -> bool {
    let parts: Vec<&str> = required.splitn(2, ':').collect();
    let (resource, action) = if parts.len() == 2 {
        (parts[0], parts[1])
    } else {
        (required, "*")
    };

    permissions.iter().any(|p| {
        p == "admin"
            || p == "*:*"
            || p == required
            || p == &format!("{resource}:*")
            || (action == "*" && p.starts_with(&format!("{resource}:")))
    })
}

/// Require a specific permission in "resource:action" format.
///
/// Checks both JWT claims and x-user-permissions header.
/// Admin role bypasses all checks.
///
/// Usage:
/// ```
///   .route_layer(middleware::from_fn(require_perm("intent:create")))
/// ```
pub fn require_perm(
    permission: &'static str,
) -> impl Fn(Request, axum::middleware::Next) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Response> + Send>,
> + Clone
       + Send {
    move |request: Request, next: axum::middleware::Next| {
        Box::pin(async move {
            let user_id = extract_user_id(&request);
            let permissions = extract_permissions(&request);

            if permissions.is_empty() {
                return (StatusCode::UNAUTHORIZED, "No permissions found").into_response();
            }

            if !has_permission(&permissions, permission) {
                tracing::warn!(
                    user_id = %user_id,
                    required = permission,
                    available = ?permissions,
                    "permission_denied"
                );
                return (
                    StatusCode::FORBIDDEN,
                    format!("Permission '{permission}' required"),
                )
                    .into_response();
            }

            next.run(request).await
        })
    }
}

/// Require a specific role (convenience wrapper).
pub fn require_role(
    role: &'static str,
) -> impl Fn(Request, axum::middleware::Next) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Response> + Send>,
> + Clone
       + Send {
    require_perm(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_has_all_permissions() {
        let perms = vec!["admin".to_string()];
        assert!(has_permission(&perms, "intent:create"));
        assert!(has_permission(&perms, "bid:create"));
        assert!(has_permission(&perms, "anything:whatever"));
    }

    #[test]
    fn wildcard_resource() {
        let perms = vec!["intent:*".to_string()];
        assert!(has_permission(&perms, "intent:create"));
        assert!(has_permission(&perms, "intent:read"));
        assert!(!has_permission(&perms, "bid:create"));
    }

    #[test]
    fn exact_match() {
        let perms = vec!["intent:create".to_string(), "market:read".to_string()];
        assert!(has_permission(&perms, "intent:create"));
        assert!(has_permission(&perms, "market:read"));
        assert!(!has_permission(&perms, "intent:delete"));
        assert!(!has_permission(&perms, "bid:create"));
    }

    #[test]
    fn global_wildcard() {
        let perms = vec!["*:*".to_string()];
        assert!(has_permission(&perms, "anything:here"));
    }

    #[test]
    fn empty_permissions() {
        let perms: Vec<String> = vec![];
        assert!(!has_permission(&perms, "intent:create"));
    }

    #[test]
    fn role_check() {
        let perms = vec!["trader".to_string()];
        assert!(has_permission(&perms, "trader"));
        assert!(!has_permission(&perms, "admin"));
    }
}
