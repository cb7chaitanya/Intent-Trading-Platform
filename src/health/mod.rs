use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Clone)]
pub struct HealthState {
    pub pg_pool: PgPool,
    pub redis_url: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<ServiceHealth>,
}

#[derive(Serialize)]
pub struct ServiceHealth {
    pub db: ComponentStatus,
    pub redis: ComponentStatus,
    pub engine: ComponentStatus,
}

#[derive(Serialize)]
pub struct ComponentStatus {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ComponentStatus {
    fn ok(latency_ms: f64) -> Self {
        Self {
            status: "ok",
            latency_ms: Some(latency_ms),
            error: None,
        }
    }

    fn err(msg: String) -> Self {
        Self {
            status: "error",
            latency_ms: None,
            error: Some(msg),
        }
    }
}

pub fn router(state: HealthState) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/health/db", get(check_db))
        .route("/health/redis", get(check_redis))
        .with_state(state)
}

/// Liveness: always returns 200 if the process is running.
async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        services: None,
    })
}

/// Readiness: checks all dependencies.
async fn readiness(
    State(state): State<HealthState>,
) -> (StatusCode, Json<HealthResponse>) {
    let db = ping_db(&state.pg_pool).await;
    let redis = ping_redis(&state.redis_url).await;

    // Engine health: check if the auction gauge is registered (proxy for engine running)
    let engine = {
        let active = crate::metrics::gauges::ACTIVE_AUCTIONS.get();
        // If we can read the gauge, the engine subsystem is initialized
        ComponentStatus {
            status: "ok",
            latency_ms: None,
            error: if active >= 0 { None } else { Some("negative gauge".to_string()) },
        }
    };

    let all_ok = db.status == "ok" && redis.status == "ok" && engine.status == "ok";

    let code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(HealthResponse {
            status: if all_ok { "ok" } else { "degraded" },
            services: Some(ServiceHealth { db, redis, engine }),
        }),
    )
}

/// Database health check.
async fn check_db(
    State(state): State<HealthState>,
) -> (StatusCode, Json<ComponentStatus>) {
    let status = ping_db(&state.pg_pool).await;
    let code = if status.status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(status))
}

/// Redis health check.
async fn check_redis(
    State(state): State<HealthState>,
) -> (StatusCode, Json<ComponentStatus>) {
    let status = ping_redis(&state.redis_url).await;
    let code = if status.status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(status))
}

async fn ping_db(pool: &PgPool) -> ComponentStatus {
    let start = std::time::Instant::now();
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => ComponentStatus::ok(start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => ComponentStatus::err(e.to_string()),
    }
}

async fn ping_redis(redis_url: &str) -> ComponentStatus {
    let start = std::time::Instant::now();
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(e) => return ComponentStatus::err(e.to_string()),
    };
    match client.get_multiplexed_async_connection().await {
        Ok(mut conn) => {
            let result: Result<String, _> = redis::cmd("PING")
                .query_async(&mut conn)
                .await;
            match result {
                Ok(_) => ComponentStatus::ok(start.elapsed().as_secs_f64() * 1000.0),
                Err(e) => ComponentStatus::err(e.to_string()),
            }
        }
        Err(e) => ComponentStatus::err(e.to_string()),
    }
}
