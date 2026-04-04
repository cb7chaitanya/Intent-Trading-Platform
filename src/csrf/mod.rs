pub mod middleware;

use std::sync::Arc;

use axum::extract::{Request, State};
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

async fn get_csrf_token(State(state): State<Arc<CsrfState>>, req: Request) -> Response {
    // Tie token to user if authenticated, otherwise "anonymous"
    let user_id = req.headers()
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous");

    let token = state.generate_token(user_id).await;

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
