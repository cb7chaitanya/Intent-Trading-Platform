use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;

use crate::signing::signer::{sign_request, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP};

#[derive(Clone)]
pub struct ProxyState {
    pub client: reqwest::Client,
    pub upstream_url: String,
    pub signing_secret: String,
}

pub async fn proxy_handler(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, StatusCode> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{}{}{}", state.upstream_url, path, query);

    let mut builder = state.client.request(method.clone(), &url);

    for (name, value) in request.headers() {
        if !matches!(name.as_str(), "host" | "connection" | "transfer-encoding") {
            builder = builder.header(name, value);
        }
    }

    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await.map_err(|_| StatusCode::BAD_REQUEST)?;

    // Sign outgoing request
    let (sig, ts, nonce) = sign_request(&state.signing_secret, method.as_str(), &path, &body_bytes);
    builder = builder
        .header(HEADER_SIGNATURE, &sig)
        .header(HEADER_TIMESTAMP, &ts)
        .header(HEADER_NONCE, &nonce);

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    let upstream_response = builder.send().await.map_err(|e| {
        tracing::error!(url = %url, error = %e, "upstream request failed");
        StatusCode::BAD_GATEWAY
    })?;

    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response_builder = Response::builder().status(status);
    for (name, value) in upstream_response.headers() {
        if !matches!(name.as_str(), "transfer-encoding" | "connection") {
            response_builder = response_builder.header(name, value);
        }
    }
    let bytes = upstream_response.bytes().await.map_err(|e| {
        tracing::error!(error = %e, "failed to read upstream body");
        StatusCode::BAD_GATEWAY
    })?;
    response_builder.body(Body::from(bytes)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
