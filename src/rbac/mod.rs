pub mod middleware;
pub mod service;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use self::service::{RbacService, Role};

#[derive(Deserialize)]
pub struct AssignRoleRequest {
    pub user_id: Uuid,
    pub role_name: String,
}

async fn assign_role(
    State(svc): State<Arc<RbacService>>,
    Json(req): Json<AssignRoleRequest>,
) -> Result<(StatusCode, &'static str), (StatusCode, String)> {
    svc.assign_role(req.user_id, &req.role_name)
        .await
        .map(|_| (StatusCode::OK, "Role assigned"))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_user_roles(
    State(svc): State<Arc<RbacService>>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Vec<Role>>, (StatusCode, String)> {
    svc.get_user_roles(user_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list_roles(
    State(svc): State<Arc<RbacService>>,
) -> Result<Json<Vec<Role>>, (StatusCode, String)> {
    svc.list_roles()
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub fn router(rbac_service: Arc<RbacService>) -> Router {
    Router::new()
        .route("/admin/roles", get(list_roles))
        .route("/admin/roles/assign", post(assign_role))
        .route("/admin/users/{user_id}/roles", get(get_user_roles))
        .with_state(rbac_service)
}
