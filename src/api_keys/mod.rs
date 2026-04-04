pub mod service;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use self::service::{ApiKeyService, CreateKeyResponse, ApiKey};

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    pub user_id: Uuid,
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Deserialize)]
pub struct RevokeKeyRequest {
    pub user_id: Uuid,
}

async fn create_key(
    State(svc): State<Arc<ApiKeyService>>,
    Json(req): Json<CreateKeyRequest>,
) -> Result<(StatusCode, Json<CreateKeyResponse>), (StatusCode, String)> {
    svc.create_key(req.user_id, &req.name, req.permissions)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list_keys(
    State(svc): State<Arc<ApiKeyService>>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Vec<ApiKey>>, (StatusCode, String)> {
    svc.list_keys(user_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn revoke_key(
    State(svc): State<Arc<ApiKeyService>>,
    Path(key_id): Path<Uuid>,
    Json(req): Json<RevokeKeyRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    svc.revoke_key(key_id, req.user_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

pub fn router(api_key_service: Arc<ApiKeyService>) -> Router {
    Router::new()
        .route("/api-keys", post(create_key))
        .route("/api-keys/{user_id}", get(list_keys))
        .route("/api-keys/{key_id}/revoke", delete(revoke_key))
        .with_state(api_key_service)
}
