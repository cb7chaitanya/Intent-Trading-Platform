use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{TransactionRecord, TxStatus, Wallet};

pub struct WalletRepository {
    pool: PgPool,
}

impl WalletRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ── Wallets ────────────────────────────────────────

    pub async fn insert_wallet(&self, wallet: &Wallet) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO wallets (id, account_id, address, chain, encrypted_key, nonce, active, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(wallet.id)
        .bind(wallet.account_id)
        .bind(&wallet.address)
        .bind(&wallet.chain)
        .bind(&wallet.encrypted_key)
        .bind(&wallet.nonce)
        .bind(wallet.active)
        .bind(wallet.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_wallet(&self, id: Uuid) -> Result<Option<Wallet>, sqlx::Error> {
        sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn find_wallet_by_account(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<Wallet>, sqlx::Error> {
        sqlx::query_as::<_, Wallet>(
            "SELECT * FROM wallets WHERE account_id = $1 AND active = TRUE ORDER BY created_at",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn find_wallet_by_address(&self, address: &str) -> Result<Option<Wallet>, sqlx::Error> {
        sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE address = $1 AND active = TRUE")
            .bind(address)
            .fetch_optional(&self.pool)
            .await
    }

    // ── Transactions ───────────────────────────────────

    pub async fn insert_transaction(&self, tx: &TransactionRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO transactions
                (id, fill_id, from_address, to_address, chain, tx_hash, amount, asset,
                 status, gas_price, gas_used, block_number, confirmations, error,
                 submitted_at, confirmed_at, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        )
        .bind(tx.id)
        .bind(tx.fill_id)
        .bind(&tx.from_address)
        .bind(&tx.to_address)
        .bind(&tx.chain)
        .bind(&tx.tx_hash)
        .bind(tx.amount)
        .bind(&tx.asset)
        .bind(&tx.status)
        .bind(tx.gas_price)
        .bind(tx.gas_used)
        .bind(tx.block_number)
        .bind(tx.confirmations)
        .bind(&tx.error)
        .bind(tx.submitted_at)
        .bind(tx.confirmed_at)
        .bind(tx.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_transaction(&self, id: Uuid) -> Result<Option<TransactionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TransactionRecord>("SELECT * FROM transactions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn find_transactions_by_fill(
        &self,
        fill_id: Uuid,
    ) -> Result<Vec<TransactionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TransactionRecord>(
            "SELECT * FROM transactions WHERE fill_id = $1 ORDER BY created_at",
        )
        .bind(fill_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn find_pending_transactions(&self) -> Result<Vec<TransactionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TransactionRecord>(
            "SELECT * FROM transactions WHERE status IN ('pending', 'submitted')
             ORDER BY created_at ASC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_tx_submitted(
        &self,
        id: Uuid,
        tx_hash: &str,
        gas_price: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE transactions SET status = 'submitted', tx_hash = $2, gas_price = $3, submitted_at = $4
             WHERE id = $1",
        )
        .bind(id)
        .bind(tx_hash)
        .bind(gas_price)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_tx_confirmed(
        &self,
        id: Uuid,
        block_number: i64,
        gas_used: i64,
        confirmations: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE transactions
             SET status = 'confirmed', block_number = $2, gas_used = $3,
                 confirmations = $4, confirmed_at = $5
             WHERE id = $1",
        )
        .bind(id)
        .bind(block_number)
        .bind(gas_used)
        .bind(confirmations)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_tx_failed(
        &self,
        id: Uuid,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE transactions SET status = 'failed', error = $2 WHERE id = $1")
            .bind(id)
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn increment_confirmations(
        &self,
        id: Uuid,
        confirmations: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE transactions SET confirmations = $2 WHERE id = $1")
            .bind(id)
            .bind(confirmations)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
