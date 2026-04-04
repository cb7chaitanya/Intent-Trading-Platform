pub mod service;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use self::service::OracleService;

pub fn router(oracle: Arc<OracleService>) -> Router {
    Router::new()
        .route("/oracle/prices", get(handler_list_prices))
        .route("/oracle/prices/{market_id}", get(handler_get_price))
        .with_state(oracle)
}

async fn handler_list_prices(
    axum::extract::State(oracle): axum::extract::State<Arc<OracleService>>,
) -> axum::Json<Vec<service::MarketPrice>> {
    axum::Json(oracle.get_all_prices().await)
}

async fn handler_get_price(
    axum::extract::State(oracle): axum::extract::State<Arc<OracleService>>,
    axum::extract::Path(market_id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<service::MarketPrice>, axum::http::StatusCode> {
    oracle
        .get_price(&market_id)
        .await
        .map(axum::Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}
