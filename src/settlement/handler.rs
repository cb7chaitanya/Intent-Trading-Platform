use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::intent::IntentStatus;

use super::engine::{SettlementEngine, SettlementError};
use super::model::{CreateTradeRequest, Trade};

#[derive(Deserialize)]
pub struct SettleIntentRequest {
    pub buyer_account_id: Uuid,
    pub seller_account_id: Uuid,
    pub asset_in: String,
    pub asset_out: String,
    pub fee_rate: f64,
}

#[derive(Serialize)]
pub struct SettleIntentResponse {
    pub intent_id: Uuid,
    pub status: String,
}

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

pub async fn settle_intent(
    State(engine): State<Arc<SettlementEngine>>,
    Path(intent_id): Path<Uuid>,
    Json(req): Json<SettleIntentRequest>,
) -> Result<Json<SettleIntentResponse>, (StatusCode, String)> {
    let asset_in = parse_asset(&req.asset_in).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let asset_out = parse_asset(&req.asset_out).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let status = engine
        .settle_intent_fills(
            intent_id,
            req.buyer_account_id,
            req.seller_account_id,
            &asset_in,
            &asset_out,
            req.fee_rate,
        )
        .await
        .map_err(map_error)?;

    Ok(Json(SettleIntentResponse {
        intent_id,
        status: format!("{status:?}"),
    }))
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
        SettlementError::TradeNotFound | SettlementError::FillNotFound => {
            (StatusCode::NOT_FOUND, e.to_string())
        }
        SettlementError::AlreadySettled => (StatusCode::CONFLICT, e.to_string()),
        SettlementError::InsufficientBalance => (StatusCode::BAD_REQUEST, e.to_string()),
        SettlementError::FeeError(_) | SettlementError::DbError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

fn parse_asset(s: &str) -> Result<crate::balances::model::Asset, String> {
    match s.to_uppercase().as_str() {
        "USDC" => Ok(crate::balances::model::Asset::USDC),
        "ETH" => Ok(crate::balances::model::Asset::ETH),
        "BTC" => Ok(crate::balances::model::Asset::BTC),
        "SOL" => Ok(crate::balances::model::Asset::SOL),
        other => Err(format!("Unknown asset: {other}")),
    }
}
