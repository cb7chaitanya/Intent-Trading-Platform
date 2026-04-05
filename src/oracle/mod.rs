pub mod service;

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use self::service::{MarketPrice, OracleService, TwapPrice};

pub fn router(oracle: Arc<OracleService>) -> Router {
    Router::new()
        .route("/oracle/prices", get(handler_list_prices))
        .route("/oracle/prices/{market_id}", get(handler_get_price))
        .route("/oracle/twap/{market_id}", get(handler_twap))
        .with_state(oracle)
}

async fn handler_list_prices(
    State(oracle): State<Arc<OracleService>>,
) -> Json<Vec<MarketPrice>> {
    Json(oracle.get_all_prices().await)
}

async fn handler_get_price(
    State(oracle): State<Arc<OracleService>>,
    Path(market_id): Path<Uuid>,
) -> Result<Json<MarketPrice>, StatusCode> {
    oracle
        .get_price(&market_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct TwapQuery {
    #[serde(default = "default_window")]
    window: i64,
}

fn default_window() -> i64 {
    300 // 5 minutes
}

async fn handler_twap(
    State(oracle): State<Arc<OracleService>>,
    Path(market_id): Path<Uuid>,
    Query(query): Query<TwapQuery>,
) -> Result<Json<TwapPrice>, StatusCode> {
    let window = query.window.clamp(10, 86400); // 10s to 24h
    oracle
        .get_twap(&market_id, window)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
