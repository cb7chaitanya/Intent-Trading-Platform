use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{AuthResponse, LoginRequest, RegisterRequest, User};
use super::service::{UserError, UserService};

pub async fn register(
    State(svc): State<Arc<UserService>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
    svc.register(req).await.map(|resp| (StatusCode::CREATED, Json(resp))).map_err(map_error)
}

pub async fn login(
    State(svc): State<Arc<UserService>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    svc.login(req).await.map(Json).map_err(map_error)
}

pub async fn get_user(
    State(svc): State<Arc<UserService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<User>, (StatusCode, String)> {
    match svc.get_user(id).await {
        Ok(Some(user)) => Ok(Json(user)),
        Ok(None) => Err((StatusCode::NOT_FOUND, "User not found".to_string())),
        Err(e) => Err(map_error(e)),
    }
}

fn map_error(e: UserError) -> (StatusCode, String) {
    match e {
        UserError::EmailTaken => (StatusCode::CONFLICT, e.to_string()),
        UserError::InvalidCredentials => (StatusCode::UNAUTHORIZED, e.to_string()),
        UserError::HashError(_) | UserError::DbError(_) | UserError::AccountError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
