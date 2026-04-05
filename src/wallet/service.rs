use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::chain::{ChainAdapter, ChainError, SettlementData, TxState};
use super::model::{TransactionRecord, TxPayload, TxStatus, Wallet};
use super::registry::ChainRegistry;
use super::repository::WalletRepository;
use super::rpc::RpcClient;
use super::signing;

// ── Errors ───────────────────────────────────────────────

#[derive(Debug)]
pub enum WalletError {
    NotFound,
    SigningError(String),
    RpcError(String),
    UnsupportedChain(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for WalletError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalletError::NotFound => write!(f, "Wallet not found"),
            WalletError::SigningError(e) => write!(f, "Signing error: {e}"),
            WalletError::RpcError(e) => write!(f, "RPC error: {e}"),
            WalletError::UnsupportedChain(c) => write!(f, "Unsupported chain: {c}"),
            WalletError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for WalletError {
    fn from(e: sqlx::Error) -> Self {
        WalletError::DbError(e)
    }
}

impl From<ChainError> for WalletError {
    fn from(e: ChainError) -> Self {
        match e {
            ChainError::Rpc(msg) => WalletError::RpcError(msg),
            ChainError::Signing(msg) => WalletError::SigningError(msg),
            ChainError::Unsupported(msg) => WalletError::UnsupportedChain(msg),
            ChainError::Other(msg) => WalletError::RpcError(msg),
        }
    }
}

// ── Service ──────────────────────────────────────────────

pub struct WalletService {
    repo: WalletRepository,
    chains: Arc<ChainRegistry>,
    /// Legacy single-chain RPC kept for backwards compatibility.
    rpc: Arc<RpcClient>,
    master_key: [u8; 32],
}

impl WalletService {
    pub fn new(
        repo: WalletRepository,
        rpc: Arc<RpcClient>,
        chains: Arc<ChainRegistry>,
        master_key: [u8; 32],
    ) -> Self {
        Self {
            repo,
            chains,
            rpc,
            master_key,
        }
    }

    /// Get the chain adapter for a given chain name.
    pub fn adapter(&self, chain: &str) -> Result<&Arc<dyn ChainAdapter>, WalletError> {
        self.chains
            .get(chain)
            .map_err(|e| WalletError::UnsupportedChain(e.to_string()))
    }

    // ── Wallet management ─────────────────────────────

    pub async fn create_wallet(
        &self,
        account_id: Uuid,
        chain: &str,
    ) -> Result<Wallet, WalletError> {
        // Verify chain is supported before generating keys
        self.adapter(chain)?;

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

    pub fn decrypt_wallet_key(&self, wallet: &Wallet) -> Result<[u8; 32], WalletError> {
        signing::decrypt_key(&wallet.encrypted_key, &wallet.nonce, &self.master_key)
            .map_err(WalletError::SigningError)
    }

    // ── Chain-routed settlement ───────────────────────

    /// Full multi-chain settlement flow:
    /// 1. Resolve chain adapter from wallet.chain
    /// 2. Build unsigned transaction
    /// 3. Decrypt private key and sign
    /// 4. Create pending DB record
    /// 5. Send via chain adapter
    /// 6. Update DB record with tx hash
    pub async fn send_settlement_tx(
        &self,
        fill_id: Uuid,
        from_wallet: &Wallet,
        to_address: &str,
        amount: i64,
        asset: &str,
    ) -> Result<TransactionRecord, WalletError> {
        let adapter = self.adapter(&from_wallet.chain)?;
        let private_key = self.decrypt_wallet_key(from_wallet)?;

        // Build unsigned tx via chain adapter
        let settlement_data = SettlementData {
            from: from_wallet.address.clone(),
            to: to_address.to_string(),
            amount: amount as u64,
            token: asset.to_string(),
            chain: from_wallet.chain.clone(),
        };

        let unsigned_tx = adapter.build_settlement_tx(&settlement_data).await?;
        let signed_tx = adapter.sign_transaction(&unsigned_tx, &private_key)?;

        // Create pending DB record
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

        // Send via chain adapter
        match adapter.send_transaction(&signed_tx).await {
            Ok(tx_hash) => {
                let fee = adapter.estimate_fees(&settlement_data).await.ok();
                let gas_price = fee.map(|f| f.total as i64).unwrap_or(0);

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
                    chain = %from_wallet.chain,
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
                    chain = %from_wallet.chain,
                    error = %error_msg,
                    "settlement_tx_send_failed"
                );
                return Err(WalletError::RpcError(error_msg));
            }
        }

        Ok(tx_record)
    }

    // ── Legacy methods (backwards compat) ─────────────

    pub fn build_tx_payload(
        &self,
        from: &str,
        to: &str,
        amount: i64,
        asset: &str,
        chain: &str,
    ) -> TxPayload {
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

    pub fn sign_tx(
        &self,
        wallet: &Wallet,
        payload: &TxPayload,
    ) -> Result<Vec<u8>, WalletError> {
        let private_key = self.decrypt_wallet_key(wallet)?;
        let tx_bytes = serde_json::to_vec(payload).unwrap_or_default();
        signing::sign_transaction(&private_key, &tx_bytes).map_err(WalletError::SigningError)
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

    pub async fn get_pending_transactions(&self) -> Result<Vec<TransactionRecord>, WalletError> {
        Ok(self.repo.find_pending_transactions().await?)
    }

    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    pub fn repo(&self) -> &WalletRepository {
        &self.repo
    }

    pub fn chains(&self) -> &ChainRegistry {
        &self.chains
    }
}
