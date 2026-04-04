use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::users::model::{LoginRequest, RegisterRequest};
use crate::users::service::{UserError, UserService};

use super::jwt;

#[derive(Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub user_id: String,
    pub email: String,
}

pub async fn register(
    State(svc): State<Arc<UserService>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<TokenResponse>), (StatusCode, String)> {
    let auth = svc.register(req).await.map_err(map_error)?;

    let token = jwt::create_token(
        auth.user_id,
        &auth.email,
        vec!["trade".to_string(), "read".to_string()],
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(TokenResponse {
            token,
            user_id: auth.user_id.to_string(),
            email: auth.email,
        }),
    ))
}

pub async fn login(
    State(svc): State<Arc<UserService>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, String)> {
    let auth = svc.login(req).await.map_err(map_error)?;

    let token = jwt::create_token(
        auth.user_id,
        &auth.email,
        vec!["trade".to_string(), "read".to_string()],
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TokenResponse {
        token,
        user_id: auth.user_id.to_string(),
        email: auth.email,
    }))
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
