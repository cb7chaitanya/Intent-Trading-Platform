use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::{Asset, Balance};

pub struct BalanceRepository {
    pool: PgPool,
}

impl BalanceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_or_create(
        &self,
        account_id: Uuid,
        asset: &Asset,
    ) -> Result<Balance, sqlx::Error> {
        let existing = sqlx::query_as::<_, Balance>(
            "SELECT * FROM balances WHERE account_id = $1 AND asset = $2",
        )
        .bind(account_id)
        .bind(asset)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(balance) = existing {
            return Ok(balance);
        }

        let balance = Balance {
            id: Uuid::new_v4(),
            account_id,
            asset: asset.clone(),
            available_balance: 0,
            locked_balance: 0,
            updated_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(balance.id)
        .bind(balance.account_id)
        .bind(&balance.asset)
        .bind(balance.available_balance)
        .bind(balance.locked_balance)
        .bind(balance.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(balance)
    }

    pub async fn update(&self, balance: &Balance) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE balances SET available_balance = $1, locked_balance = $2, updated_at = $3
             WHERE id = $4",
        )
        .bind(balance.available_balance)
        .bind(balance.locked_balance)
        .bind(balance.updated_at)
        .bind(balance.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_account_id(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<Balance>, sqlx::Error> {
        sqlx::query_as::<_, Balance>("SELECT * FROM balances WHERE account_id = $1")
            .bind(account_id)
            .fetch_all(&self.pool)
            .await
    }
}
