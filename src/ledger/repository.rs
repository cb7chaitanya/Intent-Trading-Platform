use sqlx::PgPool;
use uuid::Uuid;

use super::model::LedgerEntry;

pub struct LedgerRepository {
    pool: PgPool,
}

impl LedgerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, entry: &LedgerEntry) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(entry.id)
        .bind(entry.account_id)
        .bind(&entry.asset)
        .bind(entry.amount)
        .bind(&entry.entry_type)
        .bind(&entry.reference_type)
        .bind(entry.reference_id)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_account_id(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        sqlx::query_as::<_, LedgerEntry>(
            "SELECT * FROM ledger_entries WHERE account_id = $1 ORDER BY created_at DESC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn find_by_reference_id(
        &self,
        reference_id: Uuid,
    ) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        sqlx::query_as::<_, LedgerEntry>(
            "SELECT * FROM ledger_entries WHERE reference_id = $1 ORDER BY created_at ASC",
        )
        .bind(reference_id)
        .fetch_all(&self.pool)
        .await
    }
}
