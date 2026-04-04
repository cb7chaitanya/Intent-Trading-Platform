pub mod listener;
pub mod model;
pub mod scheduler;
pub mod service;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use self::model::{CreateTwapRequest, TwapIntent, TwapProgress};
use self::service::{TwapError, TwapService};

async fn create_twap(
    State(svc): State<Arc<TwapService>>,
    Json(req): Json<CreateTwapRequest>,
) -> Result<(StatusCode, Json<TwapIntent>), (StatusCode, String)> {
    svc.create_twap(req)
        .await
        .map(|t| (StatusCode::CREATED, Json(t)))
        .map_err(map_error)
}

async fn cancel_twap(
    State(svc): State<Arc<TwapService>>,
    Path(twap_id): Path<Uuid>,
) -> Result<Json<TwapIntent>, (StatusCode, String)> {
    // account_id would come from JWT claims in production
    svc.cancel_twap(twap_id, Uuid::nil())
        .await
        .map(Json)
        .map_err(map_error)
}

async fn get_progress(
    State(svc): State<Arc<TwapService>>,
    Path(twap_id): Path<Uuid>,
) -> Result<Json<TwapProgress>, (StatusCode, String)> {
    svc.get_progress(twap_id)
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(e: TwapError) -> (StatusCode, String) {
    match e {
        TwapError::InvalidParams(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        TwapError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        TwapError::AlreadyCancelled => (StatusCode::CONFLICT, e.to_string()),
        TwapError::DbError(_) | TwapError::IntentError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

pub fn router(twap_service: Arc<TwapService>) -> Router {
    Router::new()
        .route("/twap", post(create_twap))
        .route("/twap/{twap_id}", get(get_progress))
        .route("/twap/{twap_id}/cancel", post(cancel_twap))
        .with_state(twap_service)
}
