use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::balances::model::Asset;

use super::model::LedgerEntry;
use super::service::LedgerService;

pub async fn get_entries(
    State(svc): State<Arc<LedgerService>>,
    Path(account_id): Path<Uuid>,
) -> Result<Json<Vec<LedgerEntry>>, (StatusCode, String)> {
    svc.get_entries(account_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn get_entries_by_reference(
    State(svc): State<Arc<LedgerService>>,
    Path(reference_id): Path<Uuid>,
) -> Result<Json<Vec<LedgerEntry>>, (StatusCode, String)> {
    svc.get_entries_by_reference(reference_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
pub struct BalanceQuery {
    pub asset: Asset,
}

#[derive(Serialize)]
pub struct LedgerBalance {
    pub account_id: Uuid,
    pub asset: Asset,
    pub net_balance: i64,
}

pub async fn get_balance(
    State(svc): State<Arc<LedgerService>>,
    Path(account_id): Path<Uuid>,
    Query(query): Query<BalanceQuery>,
) -> Result<Json<LedgerBalance>, (StatusCode, String)> {
    let net_balance = svc
        .get_balance(account_id, query.asset.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(LedgerBalance {
        account_id,
        asset: query.asset,
        net_balance,
    }))
}
