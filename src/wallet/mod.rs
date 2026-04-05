pub mod chain;
pub mod confirmation;
pub mod ethereum;
pub mod model;
pub mod registry;
pub mod repository;
pub mod rpc;
pub mod service;
pub mod signing;
pub mod solana;
pub mod solana_signing;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use self::service::WalletService;

pub fn router(wallet_service: Arc<WalletService>) -> Router {
    Router::new()
        .route("/wallets", post(handler::create_wallet))
        .route("/wallets/{id}", get(handler::get_wallet))
        .route("/wallets/account/{account_id}", get(handler::get_wallets_for_account))
        .route("/transactions/{id}", get(handler::get_transaction))
        .route("/transactions/fill/{fill_id}", get(handler::get_transactions_for_fill))
        .with_state(wallet_service)
}

mod handler {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;
    use uuid::Uuid;

    use super::model::{CreateWalletRequest, WalletPublic, TransactionRecord};
    use super::service::WalletService;

    pub async fn create_wallet(
        State(svc): State<Arc<WalletService>>,
        Json(req): Json<CreateWalletRequest>,
    ) -> Result<(StatusCode, Json<WalletPublic>), (StatusCode, String)> {
        let wallet = svc.create_wallet(req.account_id, &req.chain)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok((StatusCode::CREATED, Json(WalletPublic::from(wallet))))
    }

    pub async fn get_wallet(
        State(svc): State<Arc<WalletService>>,
        Path(id): Path<Uuid>,
    ) -> Result<Json<WalletPublic>, (StatusCode, String)> {
        let wallet = svc.get_wallet(id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, "Wallet not found".to_string()))?;
        Ok(Json(WalletPublic::from(wallet)))
    }

    pub async fn get_wallets_for_account(
        State(svc): State<Arc<WalletService>>,
        Path(account_id): Path<Uuid>,
    ) -> Result<Json<Vec<WalletPublic>>, (StatusCode, String)> {
        let wallets = svc.get_wallets_for_account(account_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(Json(wallets.into_iter().map(WalletPublic::from).collect()))
    }

    pub async fn get_transaction(
        State(svc): State<Arc<WalletService>>,
        Path(id): Path<Uuid>,
    ) -> Result<Json<TransactionRecord>, (StatusCode, String)> {
        svc.get_transaction(id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map(Json)
            .ok_or((StatusCode::NOT_FOUND, "Transaction not found".to_string()))
    }

    pub async fn get_transactions_for_fill(
        State(svc): State<Arc<WalletService>>,
        Path(fill_id): Path<Uuid>,
    ) -> Result<Json<Vec<TransactionRecord>>, (StatusCode, String)> {
        svc.get_transactions_for_fill(fill_id)
            .await
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    }
}
