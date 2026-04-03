use sqlx::PgPool;
use uuid::Uuid;

use super::model::Account;

pub struct AccountRepository {
    pool: PgPool,
}

impl AccountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, account: &Account) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO accounts (id, user_id, account_type, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(account.id)
        .bind(account.user_id)
        .bind(&account.account_type)
        .bind(account.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Account>, sqlx::Error> {
        sqlx::query_as::<_, Account>(
            "SELECT id, user_id, account_type, created_at FROM accounts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn find_by_user_id(&self, user_id: Uuid) -> Result<Vec<Account>, sqlx::Error> {
        sqlx::query_as::<_, Account>(
            "SELECT id, user_id, account_type, created_at FROM accounts WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }
}
