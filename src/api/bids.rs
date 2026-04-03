use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::AppState;
use crate::models::bid::SolverBid;

#[derive(Deserialize)]
pub struct SubmitBidRequest {
    pub intent_id: Uuid,
    pub solver_id: String,
    pub amount_out: u64,
    pub fee: u64,
}

pub async fn submit_bid(
    State(state): State<AppState>,
    Json(req): Json<SubmitBidRequest>,
) -> Result<(StatusCode, Json<SolverBid>), (StatusCode, String)> {
    let mut svc = state.bid_service.lock().await;
    let bid = svc
        .submit_bid(req.intent_id, req.solver_id, req.amount_out, req.fee)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(bid)))
}
