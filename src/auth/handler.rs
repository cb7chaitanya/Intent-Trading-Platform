use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::rbac::service::RbacService;
use crate::users::model::{LoginRequest, RegisterRequest};
use crate::users::service::{UserError, UserService};

use super::jwt;

#[derive(Clone)]
pub struct AuthState {
    pub user_service: Arc<UserService>,
    pub rbac_service: Arc<RbacService>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub user_id: String,
    pub email: String,
    pub roles: Vec<String>,
}

pub async fn register(
    State(state): State<AuthState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<TokenResponse>), (StatusCode, String)> {
    let auth = state.user_service.register(req).await.map_err(map_error)?;

    // Assign default trader role
    let _ = state
        .rbac_service
        .assign_role(auth.user_id, "trader")
        .await;

    let roles = state
        .rbac_service
        .get_user_role_names(auth.user_id)
        .await
        .unwrap_or_else(|_| vec!["trader".to_string()]);

    let token = jwt::create_token(auth.user_id, &auth.email, roles.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(TokenResponse {
            token,
            user_id: auth.user_id.to_string(),
            email: auth.email,
            roles,
        }),
    ))
}

pub async fn login(
    State(state): State<AuthState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, String)> {
    let auth = state.user_service.login(req).await.map_err(map_error)?;

    let roles = state
        .rbac_service
        .get_user_role_names(auth.user_id)
        .await
        .unwrap_or_default();

    let token = jwt::create_token(auth.user_id, &auth.email, roles.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TokenResponse {
        token,
        user_id: auth.user_id.to_string(),
        email: auth.email,
        roles,
    }))
}

fn map_error(e: UserError) -> (StatusCode, String) {
    match e {
        UserError::EmailTaken => (StatusCode::CONFLICT, e.to_string()),
        UserError::WeakPassword(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        UserError::InvalidCredentials => (StatusCode::UNAUTHORIZED, e.to_string()),
        UserError::HashError(_) | UserError::DbError(_) | UserError::AccountError(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}
