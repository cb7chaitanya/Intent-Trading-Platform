use std::collections::HashMap;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use tokio::sync::Mutex;
use tower::{Layer, Service};
use uuid::Uuid;

const TOKEN_TTL_SECS: u64 = 3600; // 1 hour
const CSRF_HEADER: &str = "x-csrf-token";
const CSRF_COOKIE: &str = "csrf_token";

/// Shared state for CSRF token storage.
#[derive(Clone)]
pub struct CsrfState {
    tokens: Arc<Mutex<HashMap<String, Instant>>>,
}

impl CsrfState {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Generate a new CSRF token and store it.
    pub async fn generate_token(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let mut tokens = self.tokens.lock().await;

        // Prune expired tokens
        let cutoff = Instant::now() - std::time::Duration::from_secs(TOKEN_TTL_SECS);
        tokens.retain(|_, created| *created > cutoff);

        tokens.insert(token.clone(), Instant::now());
        token
    }

    /// Validate and consume a CSRF token (single-use).
    pub async fn validate_token(&self, token: &str) -> bool {
        let mut tokens = self.tokens.lock().await;

        match tokens.remove(token) {
            Some(created) => {
                created.elapsed().as_secs() < TOKEN_TTL_SECS
            }
            None => false,
        }
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
        CsrfService {
            inner,
            state: self.state.clone(),
        }
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
            // Only validate on state-changing methods
            let method = req.method().clone();
            if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
                return inner.call(req).await;
            }

            // Extract token from header
            let header_token = req
                .headers()
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            // Extract token from cookie (for double-submit validation)
            let cookie_token = req
                .headers()
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .and_then(|cookies| {
                    cookies.split(';').find_map(|c| {
                        let c = c.trim();
                        if c.starts_with(CSRF_COOKIE) {
                            c.splitn(2, '=').nth(1).map(|v| v.to_string())
                        } else {
                            None
                        }
                    })
                });

            let token = match header_token {
                Some(t) => t,
                None => {
                    tracing::warn!(method = %method, path = %req.uri().path(), "missing_csrf_token");
                    return Ok(
                        (StatusCode::FORBIDDEN, "Missing X-CSRF-Token header").into_response()
                    );
                }
            };

            // Double-submit check: header must match cookie
            if let Some(cookie) = &cookie_token {
                if cookie != &token {
                    tracing::warn!("csrf_token_mismatch");
                    return Ok(
                        (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response()
                    );
                }
            }

            // Validate token exists and hasn't expired
            if !state.validate_token(&token).await {
                tracing::warn!("csrf_token_invalid_or_expired");
                return Ok(
                    (StatusCode::FORBIDDEN, "Invalid or expired CSRF token").into_response()
                );
            }

            inner.call(req).await
        })
    }
}
