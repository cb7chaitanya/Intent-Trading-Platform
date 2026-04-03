use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use super::AppState;
use crate::models::bid::SolverBid;

pub async fn get_orderbook(
    State(state): State<AppState>,
    Path(intent_id): Path<Uuid>,
) -> Json<Vec<SolverBid>> {
    let svc = state.bid_service.lock().await;
    Json(svc.build_orderbook(&intent_id))
}
