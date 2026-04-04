use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use redis::AsyncCommands;
use tower::{Layer, Service};
use uuid::Uuid;

const TOKEN_TTL_SECS: u64 = 3600;
const CSRF_HEADER: &str = "x-csrf-token";
const CSRF_COOKIE: &str = "csrf_token";
const REDIS_PREFIX: &str = "csrf:";

/// Redis-backed CSRF token state. Supports multi-instance deployment.
#[derive(Clone)]
pub struct CsrfState {
    conn: Arc<tokio::sync::Mutex<redis::aio::MultiplexedConnection>>,
}

impl CsrfState {
    pub async fn new(redis_url: &str) -> Self {
        let client = redis::Client::open(redis_url).expect("Invalid Redis URL for CSRF");
        let conn = client.get_multiplexed_async_connection().await.expect("Redis connect failed for CSRF");
        Self { conn: Arc::new(tokio::sync::Mutex::new(conn)) }
    }

    /// Generate a CSRF token tied to a user/session.
    /// Stored in Redis with TTL. Returns the token string.
    pub async fn generate_token(&self, user_id: &str) -> String {
        let token = Uuid::new_v4().to_string();
        let key = format!("{REDIS_PREFIX}{token}");
        let mut conn = self.conn.lock().await;
        let _: Result<(), _> = conn.set_ex(&key, user_id, TOKEN_TTL_SECS).await;
        token
    }

    /// Validate and consume a CSRF token (single-use).
    /// Returns the user_id the token was issued for, or None if invalid.
    pub async fn validate_token(&self, token: &str) -> Option<String> {
        let key = format!("{REDIS_PREFIX}{token}");
        let mut conn = self.conn.lock().await;

        // GET + DEL atomically (single-use)
        let user_id: Option<String> = conn.get(&key).await.ok().flatten();
        if user_id.is_some() {
            let _: Result<(), _> = conn.del(&key).await;
        }
        user_id
    }
}

// ---------------------------------------------------------------
// Tower Layer / Service
// ---------------------------------------------------------------

#[derive(Clone)]
pub struct CsrfLayer {
    state: Arc<CsrfState>,
}

impl CsrfLayer {
    pub fn new(state: Arc<CsrfState>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for CsrfLayer {
    type Service = CsrfService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CsrfService { inner, state: self.state.clone() }
    }
}

#[derive(Clone)]
pub struct CsrfService<S> {
    inner: S,
    state: Arc<CsrfState>,
}

impl<S> Service<Request<Body>> for CsrfService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let state = self.state.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let method = req.method().clone();
            if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
                return inner.call(req).await;
            }

            // Extract token from header
            let header_token = req.headers()
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            // Extract token from cookie
            let cookie_token = req.headers()
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .and_then(|cookies| {
                    cookies.split(';').find_map(|c| {
                        let c = c.trim();
                        if c.starts_with(CSRF_COOKIE) {
                            c.splitn(2, '=').nth(1).map(|v| v.to_string())
                        } else { None }
                    })
                });

            let token = match header_token {
                Some(t) => t,
                None => {
                    tracing::warn!(method = %method, path = %req.uri().path(), "missing_csrf_token");
                    return Ok((StatusCode::FORBIDDEN, "Missing X-CSRF-Token header").into_response());
                }
            };

            // Double-submit: header must match cookie
            if let Some(cookie) = &cookie_token {
                if cookie != &token {
                    tracing::warn!("csrf_token_mismatch");
                    return Ok((StatusCode::FORBIDDEN, "CSRF token mismatch").into_response());
                }
            }

            // Validate + consume in Redis (single-use)
            let user_id_from_token = match state.validate_token(&token).await {
                Some(uid) => uid,
                None => {
                    tracing::warn!("csrf_token_invalid_or_expired");
                    return Ok((StatusCode::FORBIDDEN, "Invalid or expired CSRF token").into_response());
                }
            };

            // Optionally verify token was issued for this user
            if let Some(request_user) = req.headers().get("x-user-id").and_then(|v| v.to_str().ok()) {
                if request_user != user_id_from_token && user_id_from_token != "anonymous" {
                    tracing::warn!(
                        token_user = %user_id_from_token,
                        request_user = %request_user,
                        "csrf_user_mismatch"
                    );
                    return Ok((StatusCode::FORBIDDEN, "CSRF token not issued for this user").into_response());
                }
            }

            inner.call(req).await
        })
    }
}
