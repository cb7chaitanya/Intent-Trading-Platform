use std::collections::HashMap;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use tokio::sync::Mutex;
use tower::{Layer, Service};

fn window_secs() -> u64 {
    crate::config::get().rate_limit_window_secs
}

fn max_requests() -> u64 {
    crate::config::get().rate_limit_per_minute
}

#[derive(Clone)]
pub struct RateLimiter {
    counters: Arc<Mutex<HashMap<String, (u64, std::time::Instant)>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn extract_key(req: &Request<Body>) -> String {
        if let Some(key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
            return key.to_string();
        }
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string()
    }

    async fn check(&self, key: &str) -> Result<(u64, u64), StatusCode> {
        let now = std::time::Instant::now();
        let mut counters = self.counters.lock().await;
        let entry = counters.entry(key.to_string()).or_insert((0, now));

        if now.duration_since(entry.1).as_secs() >= window_secs() {
            *entry = (0, now);
        }
        entry.0 += 1;

        if entry.0 > max_requests() {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        Ok((max_requests() - entry.0, window_secs()))
    }
}

// Tower Layer
#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: RateLimiter,
}

impl RateLimitLayer {
    pub fn new(limiter: RateLimiter) -> Self {
        Self { limiter }
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: self.limiter.clone(),
        }
    }
}

// Tower Service
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: RateLimiter,
}

impl<S> Service<Request<Body>> for RateLimitService<S>
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
        let limiter = self.limiter.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let key = RateLimiter::extract_key(&req);

            let (remaining, window) = match limiter.check(&key).await {
                Ok(v) => v,
                Err(status) => return Ok(status.into_response()),
            };

            tracing::info!(key = %key, remaining, "rate limit check passed");

            let mut response = inner.call(req).await?;
            let headers = response.headers_mut();
            let _ = headers.insert("x-ratelimit-limit", max_requests().into());
            let _ = headers.insert("x-ratelimit-remaining", remaining.into());
            let _ = headers.insert("x-ratelimit-window", window.into());

            Ok(response)
        })
    }
}
