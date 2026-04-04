use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use redis::aio::MultiplexedConnection;
use tower::{Layer, Service};

/// Per-route rate limit configuration.
struct RouteLimit {
    max_requests: u64,
    window_secs: u64,
}

fn route_limit(path: &str) -> RouteLimit {
    let cfg = crate::config::get();
    let default = RouteLimit {
        max_requests: cfg.rate_limit_per_minute,
        window_secs: cfg.rate_limit_window_secs,
    };

    // Tighter limits for write endpoints
    if path.starts_with("/intents") || path.starts_with("/bids") {
        return RouteLimit { max_requests: 30, window_secs: 60 };
    }
    if path.starts_with("/balances/deposit") || path.starts_with("/balances/withdraw") {
        return RouteLimit { max_requests: 10, window_secs: 60 };
    }
    if path.starts_with("/auth/") {
        return RouteLimit { max_requests: 5, window_secs: 60 };
    }

    default
}

/// Redis-backed sliding window rate limiter using sorted sets.
#[derive(Clone)]
pub struct RateLimiter {
    conn: MultiplexedConnection,
}

impl RateLimiter {
    pub async fn new(redis_url: &str) -> Self {
        let client = redis::Client::open(redis_url).expect("Invalid Redis URL for rate limiter");
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("Failed to connect Redis for rate limiter");
        Self { conn }
    }

    fn extract_identity(req: &Request<Body>) -> String {
        // Prefer user ID from JWT (injected by auth middleware)
        if let Some(uid) = req.headers().get("x-user-id").and_then(|v| v.to_str().ok()) {
            return uid.to_string();
        }
        if let Some(key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
            return key.to_string();
        }
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string()
    }

    /// Sliding window check using Redis sorted set.
    ///
    /// Key: rate_limit:{identity}:{route_prefix}
    /// Members: unique request IDs scored by timestamp (microseconds)
    ///
    /// Algorithm:
    ///   1. ZREMRANGEBYSCORE to remove entries outside the window
    ///   2. ZCARD to count entries in the window
    ///   3. If under limit, ZADD the new request
    ///   4. EXPIRE the key as a safety net
    async fn check(
        &self,
        identity: &str,
        route: &str,
        limit: &RouteLimit,
    ) -> Result<RateLimitResult, StatusCode> {
        let route_prefix = route.split('/').take(2).collect::<Vec<_>>().join("/");
        let key = format!("rate_limit:{identity}:{route_prefix}");
        let now_micros = chrono::Utc::now().timestamp_micros();
        let window_start = now_micros - (limit.window_secs as i64 * 1_000_000);

        let mut conn = self.conn.clone();

        // Pipeline: remove old + count + add new + expire
        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("ZREMRANGEBYSCORE").arg(&key).arg("-inf").arg(window_start).ignore()
            .cmd("ZCARD").arg(&key)
            .cmd("ZADD").arg(&key).arg(now_micros).arg(now_micros).ignore()
            .cmd("EXPIRE").arg(&key).arg(limit.window_secs * 2).ignore();

        let results: Vec<redis::Value> = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Redis rate limit pipeline failed");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // ZCARD result is at index 1 (after ZREMRANGEBYSCORE which was ignored)
        let count = match &results[..] {
            [_, redis::Value::Int(n), ..] => *n as u64,
            _ => 0,
        };

        if count >= limit.max_requests {
            // Over limit — remove the entry we just added
            let _: Result<(), _> = redis::cmd("ZREM")
                .arg(&key)
                .arg(now_micros)
                .query_async(&mut conn)
                .await;

            let reset_secs = limit.window_secs;
            tracing::warn!(
                identity = %identity,
                route = %route_prefix,
                count,
                limit = limit.max_requests,
                "rate_limit_exceeded"
            );

            return Err(StatusCode::TOO_MANY_REQUESTS);
        }

        let remaining = limit.max_requests - count - 1; // -1 for the request we just added
        let reset_secs = limit.window_secs;

        Ok(RateLimitResult {
            limit: limit.max_requests,
            remaining,
            reset_secs,
        })
    }
}

struct RateLimitResult {
    limit: u64,
    remaining: u64,
    reset_secs: u64,
}

// ---------------------------------------------------------------
// Tower Layer / Service
// ---------------------------------------------------------------

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
            let identity = RateLimiter::extract_identity(&req);
            let path = req.uri().path().to_string();
            let limit = route_limit(&path);

            let result = match limiter.check(&identity, &path, &limit).await {
                Ok(r) => r,
                Err(status) => {
                    let mut resp = status.into_response();
                    let h = resp.headers_mut();
                    let _ = h.insert("x-ratelimit-limit", limit.max_requests.into());
                    let _ = h.insert("x-ratelimit-remaining", 0u64.into());
                    let _ = h.insert("x-ratelimit-reset", limit.window_secs.into());
                    let _ = h.insert("retry-after", limit.window_secs.into());
                    return Ok(resp);
                }
            };

            let mut response = inner.call(req).await?;
            let h = response.headers_mut();
            let _ = h.insert("x-ratelimit-limit", result.limit.into());
            let _ = h.insert("x-ratelimit-remaining", result.remaining.into());
            let _ = h.insert("x-ratelimit-reset", result.reset_secs.into());

            Ok(response)
        })
    }
}
