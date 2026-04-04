use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use redis::AsyncCommands;
use tower::{Layer, Service};

use super::signer::{compute_signature, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP};

const MAX_AGE_SECS: i64 = 30;
const NONCE_TTL_SECS: u64 = 60;
const NONCE_PREFIX: &str = "nonce:";

/// Tower Layer that verifies HMAC signatures on incoming requests.
#[derive(Clone)]
pub struct VerifySignatureLayer {
    secret: String,
    redis_conn: Arc<tokio::sync::Mutex<redis::aio::MultiplexedConnection>>,
}

impl VerifySignatureLayer {
    pub async fn new(secret: &str, redis_url: &str) -> Self {
        let client = redis::Client::open(redis_url).expect("Invalid Redis URL for signature verifier");
        let conn = client.get_multiplexed_async_connection().await.expect("Redis connect failed for verifier");
        Self {
            secret: secret.to_string(),
            redis_conn: Arc::new(tokio::sync::Mutex::new(conn)),
        }
    }
}

impl<S> Layer<S> for VerifySignatureLayer {
    type Service = VerifySignatureService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        VerifySignatureService {
            inner,
            secret: self.secret.clone(),
            redis_conn: self.redis_conn.clone(),
        }
    }
}

#[derive(Clone)]
pub struct VerifySignatureService<S> {
    inner: S,
    secret: String,
    redis_conn: Arc<tokio::sync::Mutex<redis::aio::MultiplexedConnection>>,
}

impl<S> Service<Request<Body>> for VerifySignatureService<S>
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
        let secret = self.secret.clone();
        let redis_conn = self.redis_conn.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Extract headers
            let signature = req.headers().get(HEADER_SIGNATURE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            let timestamp_str = req.headers().get(HEADER_TIMESTAMP).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            let nonce = req.headers().get(HEADER_NONCE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());

            let (Some(signature), Some(timestamp_str), Some(nonce)) = (signature, timestamp_str, nonce) else {
                return Ok((StatusCode::UNAUTHORIZED, "Missing signature headers").into_response());
            };

            let timestamp: i64 = match timestamp_str.parse() {
                Ok(t) => t,
                Err(_) => return Ok((StatusCode::BAD_REQUEST, "Invalid timestamp").into_response()),
            };

            // Check age
            let now = chrono::Utc::now().timestamp();
            if (now - timestamp).abs() > MAX_AGE_SECS {
                tracing::warn!(age = now - timestamp, "signature_too_old");
                return Ok((StatusCode::UNAUTHORIZED, "Request too old").into_response());
            }

            // Check nonce replay
            let nonce_key = format!("{NONCE_PREFIX}{nonce}");
            {
                let mut conn = redis_conn.lock().await;
                let exists: bool = conn.exists(&nonce_key).await.unwrap_or(false);
                if exists {
                    tracing::warn!(nonce = %nonce, "nonce_replay_detected");
                    return Ok((StatusCode::UNAUTHORIZED, "Nonce already used").into_response());
                }
                // Store nonce to prevent replay
                let _: Result<(), _> = conn.set_ex(&nonce_key, "1", NONCE_TTL_SECS).await;
            }

            // Read body for signature verification
            let method = req.method().to_string();
            let path = req.uri().path().to_string();

            let (parts, body) = req.into_parts();
            let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
                .await
                .unwrap_or_default();

            // Verify signature
            let expected = compute_signature(&secret, &method, &path, &body_bytes, timestamp, &nonce);
            if signature != expected {
                tracing::warn!(path = %path, "signature_mismatch");
                return Ok((StatusCode::UNAUTHORIZED, "Invalid signature").into_response());
            }

            // Reconstruct request with consumed body
            let req = Request::from_parts(parts, Body::from(body_bytes));
            inner.call(req).await
        })
    }
}
