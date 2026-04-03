use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

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
