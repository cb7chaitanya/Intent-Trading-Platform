pub mod middleware;

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;

use self::middleware::CsrfState;

#[derive(Serialize)]
struct CsrfTokenResponse {
    token: String,
}

async fn get_csrf_token(State(state): State<Arc<CsrfState>>) -> Response {
    let token = state.generate_token().await;

    let cookie = format!(
        "csrf_token={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=3600",
        token
    );

    (
        StatusCode::OK,
        [(SET_COOKIE, cookie)],
        Json(CsrfTokenResponse { token }),
    )
        .into_response()
}

pub fn router(csrf_state: Arc<CsrfState>) -> axum::Router {
    axum::Router::new()
        .route("/csrf-token", get(get_csrf_token))
        .with_state(csrf_state)
}
