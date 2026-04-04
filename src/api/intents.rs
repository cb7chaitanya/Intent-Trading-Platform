use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::AppState;
use crate::models::intent::Intent;
use crate::services::intent_service::IntentError;

#[derive(Deserialize)]
pub struct CreateIntentRequest {
    pub user_id: String,
    pub account_id: Uuid,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub deadline: i64,
}

pub async fn create_intent(
    State(state): State<AppState>,
    Json(req): Json<CreateIntentRequest>,
) -> Result<(StatusCode, Json<Intent>), (StatusCode, String)> {
    let mut svc = state.intent_service.lock().await;
    let intent = svc
        .create_intent(
            req.user_id,
            req.account_id,
            req.token_in,
            req.token_out,
            req.amount_in,
            req.min_amount_out,
            req.deadline,
        )
        .await
        .map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(intent)))
}

pub async fn list_intents(
    State(state): State<AppState>,
) -> Json<Vec<Intent>> {
    let svc = state.intent_service.lock().await;
    Json(svc.list_intents().await)
}

pub async fn get_intent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Intent>, StatusCode> {
    let svc = state.intent_service.lock().await;
    svc.get_intent(&id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

fn map_error(e: IntentError) -> (StatusCode, String) {
    match e {
        IntentError::InsufficientBalance => (StatusCode::BAD_REQUEST, e.to_string()),
        IntentError::InvalidAsset(_) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        IntentError::RiskRejected(_) => (StatusCode::FORBIDDEN, e.to_string()),
        IntentError::IntentNotFound => (StatusCode::NOT_FOUND, e.to_string()),
        IntentError::StorageError(_) | IntentError::RedisError(_) | IntentError::BalanceError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
