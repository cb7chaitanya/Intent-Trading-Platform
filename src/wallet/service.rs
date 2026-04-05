use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::model::{TransactionRecord, TxPayload, TxStatus, Wallet};
use super::repository::WalletRepository;
use super::rpc::RpcClient;
use super::signing;

#[derive(Debug)]
pub enum WalletError {
    NotFound,
    SigningError(String),
    RpcError(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for WalletError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalletError::NotFound => write!(f, "Wallet not found"),
            WalletError::SigningError(e) => write!(f, "Signing error: {e}"),
            WalletError::RpcError(e) => write!(f, "RPC error: {e}"),
            WalletError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for WalletError {
    fn from(e: sqlx::Error) -> Self {
        WalletError::DbError(e)
    }
}

pub struct WalletService {
    repo: WalletRepository,
    rpc: Arc<RpcClient>,
    master_key: [u8; 32],
}

impl WalletService {
    pub fn new(repo: WalletRepository, rpc: Arc<RpcClient>, master_key: [u8; 32]) -> Self {
        Self { repo, rpc, master_key }
    }

    // ── Wallet management ─────────────────────────────

    /// Generate a new wallet: create keypair, encrypt private key, store in DB.
    pub async fn create_wallet(
        &self,
        account_id: Uuid,
        chain: &str,
    ) -> Result<Wallet, WalletError> {
        let (private_key, address) = signing::generate_keypair();
        let (encrypted_key, nonce) = signing::encrypt_key(&private_key, &self.master_key);

        let wallet = Wallet {
            id: Uuid::new_v4(),
            account_id,
            address,
            chain: chain.to_string(),
            encrypted_key,
            nonce,
            active: true,
            created_at: Utc::now(),
        };

        self.repo.insert_wallet(&wallet).await?;
        Ok(wallet)
    }

    pub async fn get_wallet(&self, id: Uuid) -> Result<Option<Wallet>, WalletError> {
        Ok(self.repo.find_wallet(id).await?)
    }

    pub async fn get_wallets_for_account(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<Wallet>, WalletError> {
        Ok(self.repo.find_wallet_by_account(account_id).await?)
    }

    /// Decrypt and return the private key for a wallet (internal use only).
    fn decrypt_wallet_key(&self, wallet: &Wallet) -> Result<[u8; 32], WalletError> {
        signing::decrypt_key(&wallet.encrypted_key, &wallet.nonce, &self.master_key)
            .map_err(WalletError::SigningError)
    }

    // ── Transaction building and signing ──────────────

    /// Build a transaction payload for a fill settlement.
    pub fn build_tx_payload(
        &self,
        from: &str,
        to: &str,
        amount: i64,
        asset: &str,
        chain: &str,
    ) -> TxPayload {
        // In production, encode contract call data (ERC-20 transfer, etc.)
        let data = serde_json::to_vec(&serde_json::json!({
            "method": "transfer",
            "to": to,
            "amount": amount,
            "asset": asset,
        }))
        .unwrap_or_default();

        TxPayload {
            from: from.to_string(),
            to: to.to_string(),
            value: amount,
            asset: asset.to_string(),
            chain: chain.to_string(),
            data,
        }
    }

    /// Sign a transaction with the wallet's private key.
    pub fn sign_tx(
        &self,
        wallet: &Wallet,
        payload: &TxPayload,
    ) -> Result<Vec<u8>, WalletError> {
        let private_key = self.decrypt_wallet_key(wallet)?;
        let tx_bytes = serde_json::to_vec(payload).unwrap_or_default();
        signing::sign_transaction(&private_key, &tx_bytes).map_err(WalletError::SigningError)
    }

    // ── Send and track ────────────────────────────────

    /// Full flow: build payload, sign, send via RPC, create DB record.
    pub async fn send_settlement_tx(
        &self,
        fill_id: Uuid,
        from_wallet: &Wallet,
        to_address: &str,
        amount: i64,
        asset: &str,
    ) -> Result<TransactionRecord, WalletError> {
        let payload = self.build_tx_payload(
            &from_wallet.address,
            to_address,
            amount,
            asset,
            &from_wallet.chain,
        );

        let signature = self.sign_tx(from_wallet, &payload)?;
        let signed_hex = format!("0x{}", hex::encode(&signature));

        // Create pending transaction record
        let tx_id = Uuid::new_v4();
        let now = Utc::now();
        let mut tx_record = TransactionRecord {
            id: tx_id,
            fill_id: Some(fill_id),
            from_address: from_wallet.address.clone(),
            to_address: to_address.to_string(),
            chain: from_wallet.chain.clone(),
            tx_hash: None,
            amount,
            asset: asset.to_string(),
            status: TxStatus::Pending,
            gas_price: None,
            gas_used: None,
            block_number: None,
            confirmations: 0,
            error: None,
            submitted_at: None,
            confirmed_at: None,
            created_at: now,
        };
        self.repo.insert_transaction(&tx_record).await?;

        // Send via RPC
        match self.rpc.send_raw_transaction(&signed_hex).await {
            Ok(tx_hash) => {
                let gas_price = self.rpc.gas_price().await.unwrap_or(0);
                self.repo
                    .update_tx_submitted(tx_id, &tx_hash, gas_price)
                    .await?;
                tx_record.tx_hash = Some(tx_hash);
                tx_record.status = TxStatus::Submitted;
                tx_record.gas_price = Some(gas_price);
                tx_record.submitted_at = Some(Utc::now());

                tracing::info!(
                    tx_id = %tx_id,
                    fill_id = %fill_id,
                    tx_hash = ?tx_record.tx_hash,
                    "settlement_tx_submitted"
                );
            }
            Err(e) => {
                let error_msg = e.to_string();
                self.repo.update_tx_failed(tx_id, &error_msg).await?;
                tx_record.status = TxStatus::Failed;
                tx_record.error = Some(error_msg.clone());

                tracing::error!(
                    tx_id = %tx_id,
                    fill_id = %fill_id,
                    error = %error_msg,
                    "settlement_tx_send_failed"
                );
                return Err(WalletError::RpcError(error_msg));
            }
        }

        Ok(tx_record)
    }

    // ── Read ──────────────────────────────────────────

    pub async fn get_transaction(
        &self,
        id: Uuid,
    ) -> Result<Option<TransactionRecord>, WalletError> {
        Ok(self.repo.find_transaction(id).await?)
    }

    pub async fn get_transactions_for_fill(
        &self,
        fill_id: Uuid,
    ) -> Result<Vec<TransactionRecord>, WalletError> {
        Ok(self.repo.find_transactions_by_fill(fill_id).await?)
    }

    pub async fn get_pending_transactions(
        &self,
    ) -> Result<Vec<TransactionRecord>, WalletError> {
        Ok(self.repo.find_pending_transactions().await?)
    }

    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    pub fn repo(&self) -> &WalletRepository {
        &self.repo
    }
}
