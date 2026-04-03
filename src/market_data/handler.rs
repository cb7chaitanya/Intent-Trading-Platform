use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{Candle, CandlesQuery, MarketTrade, OrderBookSnapshot, TradesQuery};
use super::service::{MarketDataError, MarketDataService};

pub async fn get_trades(
    State(svc): State<Arc<MarketDataService>>,
    Path(market_id): Path<Uuid>,
    Query(query): Query<TradesQuery>,
) -> Result<Json<Vec<MarketTrade>>, (StatusCode, String)> {
    svc.get_trades(market_id, query.limit)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn get_orderbook(
    State(svc): State<Arc<MarketDataService>>,
    Path(market_id): Path<Uuid>,
) -> Result<Json<OrderBookSnapshot>, (StatusCode, String)> {
    svc.get_orderbook_snapshot(market_id)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn get_candles(
    State(svc): State<Arc<MarketDataService>>,
    Path(market_id): Path<Uuid>,
    Query(query): Query<CandlesQuery>,
) -> Result<Json<Vec<Candle>>, (StatusCode, String)> {
    svc.generate_candles(market_id, &query.interval)
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(e: MarketDataError) -> (StatusCode, String) {
    match e {
        MarketDataError::InvalidInterval(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        MarketDataError::DbError(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
