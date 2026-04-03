use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{Balance, DepositRequest, WithdrawRequest};
use super::service::{BalanceError, BalanceService};

pub async fn deposit(
    State(svc): State<Arc<BalanceService>>,
    Json(req): Json<DepositRequest>,
) -> Result<Json<Balance>, (StatusCode, String)> {
    svc.deposit(req.account_id, req.asset, req.amount)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn withdraw(
    State(svc): State<Arc<BalanceService>>,
    Json(req): Json<WithdrawRequest>,
) -> Result<Json<Balance>, (StatusCode, String)> {
    svc.withdraw(req.account_id, req.asset, req.amount)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn get_balances(
    State(svc): State<Arc<BalanceService>>,
    Path(account_id): Path<Uuid>,
) -> Result<Json<Vec<Balance>>, (StatusCode, String)> {
    svc.get_balances(account_id)
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(e: BalanceError) -> (StatusCode, String) {
    match e {
        BalanceError::InsufficientBalance | BalanceError::InsufficientLockedBalance => {
            (StatusCode::BAD_REQUEST, e.to_string())
        }
        BalanceError::InvalidAmount => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        BalanceError::DbError(_) | BalanceError::LedgerError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
