pub mod middleware;

use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use futures_util::future::BoxFuture;
use sqlx::PgPool;
use tower::{Layer, Service};

/// Tower Layer that wraps services with idempotency checking.
#[derive(Clone)]
pub struct IdempotencyLayer {
    pool: PgPool,
}

impl IdempotencyLayer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = IdempotencyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IdempotencyService {
            inner,
            pool: self.pool.clone(),
        }
    }
}

/// Tower Service that checks for idempotency keys before processing.
#[derive(Clone)]
pub struct IdempotencyService<S> {
    inner: S,
    pool: PgPool,
}

impl<S> Service<Request<Body>> for IdempotencyService<S>
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
        let pool = self.pool.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Only process mutating methods
            let method = req.method().clone();
            if method != axum::http::Method::POST
                && method != axum::http::Method::PUT
                && method != axum::http::Method::PATCH
            {
                return inner.call(req).await;
            }

            // Extract idempotency key
            let idem_key = req
                .headers()
                .get("idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let Some(idem_key) = idem_key else {
                // No key — process normally
                return inner.call(req).await;
            };

            // Extract user context (from JWT middleware)
            let user_id = req
                .headers()
                .get("x-user-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            // Build request hash from method + path + body fingerprint
            let path = req.uri().path().to_string();
            let request_hash = format!("{method}:{path}:{user_id}");

            // Check if we already processed this key
            match middleware::lookup(&pool, &idem_key).await {
                Some(cached) => {
                    // Verify the request hash matches (same key must be same request)
                    if cached.request_hash != request_hash {
                        tracing::warn!(
                            key = %idem_key,
                            "Idempotency key reused for different request"
                        );
                        return Ok(middleware::conflict_response());
                    }

                    tracing::debug!(key = %idem_key, "Returning cached idempotent response");
                    Ok(middleware::build_cached_response(&cached))
                }
                None => {
                    // Process the request
                    let response = inner.call(req).await?;

                    // Capture response for storage
                    let status = response.status().as_u16() as i32;
                    let (parts, body) = response.into_parts();

                    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

                    // Store the response (best-effort, don't fail the request)
                    let _ = middleware::store(
                        &pool,
                        &idem_key,
                        &user_id,
                        &request_hash,
                        status,
                        &body_str,
                    )
                    .await;

                    // Reconstruct response
                    Ok(Response::from_parts(parts, Body::from(body_bytes)))
                }
            }
        })
    }
}
