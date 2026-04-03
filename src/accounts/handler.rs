use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{Account, CreateAccountRequest};
use super::service::AccountService;

pub async fn create_account(
    State(svc): State<Arc<AccountService>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<Account>), (StatusCode, String)> {
    svc.create_account(req.user_id)
        .await
        .map(|acc| (StatusCode::CREATED, Json(acc)))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn get_accounts(
    State(svc): State<Arc<AccountService>>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Vec<Account>>, (StatusCode, String)> {
    svc.get_accounts(user_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
