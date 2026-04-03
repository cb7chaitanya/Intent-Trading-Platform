use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{CreateMarketRequest, Market};
use super::service::{MarketError, MarketService};

pub async fn create_market(
    State(svc): State<Arc<MarketService>>,
    Json(req): Json<CreateMarketRequest>,
) -> Result<(StatusCode, Json<Market>), (StatusCode, String)> {
    svc.create_market(req)
        .await
        .map(|m| (StatusCode::CREATED, Json(m)))
        .map_err(map_error)
}

pub async fn get_market(
    State(svc): State<Arc<MarketService>>,
    Path(market_id): Path<Uuid>,
) -> Result<Json<Market>, (StatusCode, String)> {
    svc.get_market(market_id)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn list_markets(
    State(svc): State<Arc<MarketService>>,
) -> Result<Json<Vec<Market>>, (StatusCode, String)> {
    svc.list_markets()
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(e: MarketError) -> (StatusCode, String) {
    match e {
        MarketError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        MarketError::DbError(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
