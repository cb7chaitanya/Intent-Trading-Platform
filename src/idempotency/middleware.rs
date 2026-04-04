use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;

#[derive(Debug, sqlx::FromRow)]
pub struct CachedResponse {
    pub key: String,
    pub request_hash: String,
    pub status_code: i32,
    pub response_body: String,
}

/// Look up a cached response by idempotency key.
pub async fn lookup(pool: &PgPool, key: &str) -> Option<CachedResponse> {
    sqlx::query_as::<_, CachedResponse>(
        "SELECT key, request_hash, status_code, response_body
         FROM idempotency_keys WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// Store a response for an idempotency key.
pub async fn store(
    pool: &PgPool,
    key: &str,
    user_id: &str,
    request_hash: &str,
    status_code: i32,
    response_body: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO idempotency_keys (key, user_id, request_hash, status_code, response_body)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .bind(user_id)
    .bind(request_hash)
    .bind(status_code)
    .bind(response_body)
    .execute(pool)
    .await?;

    tracing::debug!(key = key, "Idempotency key stored");
    Ok(())
}

/// Build an Axum response from a cached entry.
pub fn build_cached_response(cached: &CachedResponse) -> Response {
    let status = StatusCode::from_u16(cached.status_code as u16)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut response = Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("x-idempotent-replay", "true")
        .body(Body::from(cached.response_body.clone()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());

    response
}

/// Response for when a key is reused with a different request.
pub fn conflict_response() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        "Idempotency key already used for a different request",
    )
        .into_response()
}
