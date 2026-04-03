use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::engine::{SettlementEngine, SettlementError};
use super::model::{CreateTradeRequest, Trade};

pub async fn create_trade(
    State(engine): State<Arc<SettlementEngine>>,
    Json(req): Json<CreateTradeRequest>,
) -> Result<(StatusCode, Json<Trade>), (StatusCode, String)> {
    engine
        .create_trade(req)
        .await
        .map(|t| (StatusCode::CREATED, Json(t)))
        .map_err(map_error)
}

pub async fn settle_trade(
    State(engine): State<Arc<SettlementEngine>>,
    Path(trade_id): Path<Uuid>,
) -> Result<Json<Trade>, (StatusCode, String)> {
    engine
        .settle_trade(trade_id)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn get_trade(
    State(engine): State<Arc<SettlementEngine>>,
    Path(trade_id): Path<Uuid>,
) -> Result<Json<Trade>, (StatusCode, String)> {
    match engine.get_trade(trade_id).await {
        Ok(Some(trade)) => Ok(Json(trade)),
        Ok(None) => Err((StatusCode::NOT_FOUND, "Trade not found".to_string())),
        Err(e) => Err(map_error(e)),
    }
}

fn map_error(e: SettlementError) -> (StatusCode, String) {
    match e {
        SettlementError::TradeNotFound => (StatusCode::NOT_FOUND, e.to_string()),
        SettlementError::AlreadySettled => (StatusCode::CONFLICT, e.to_string()),
        SettlementError::InsufficientBalance => (StatusCode::BAD_REQUEST, e.to_string()),
        SettlementError::FeeError(_) | SettlementError::DbError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
