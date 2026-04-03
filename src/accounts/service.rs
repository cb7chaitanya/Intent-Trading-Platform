use chrono::Utc;
use uuid::Uuid;

use super::model::{Account, AccountType};
use super::repository::AccountRepository;

#[derive(Debug)]
pub enum AccountError {
    DbError(sqlx::Error),
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for AccountError {
    fn from(e: sqlx::Error) -> Self {
        AccountError::DbError(e)
    }
}

pub struct AccountService {
    repo: AccountRepository,
}

impl AccountService {
    pub fn new(repo: AccountRepository) -> Self {
        Self { repo }
    }

    pub async fn create_default_account(&self, user_id: Uuid) -> Result<Account, AccountError> {
        let account = Account {
            id: Uuid::new_v4(),
            user_id,
            account_type: AccountType::Spot,
            created_at: Utc::now(),
        };
        self.repo.insert(&account).await?;
        Ok(account)
    }

    pub async fn create_account(&self, user_id: Uuid) -> Result<Account, AccountError> {
        self.create_default_account(user_id).await
    }

    pub async fn get_accounts(&self, user_id: Uuid) -> Result<Vec<Account>, AccountError> {
        Ok(self.repo.find_by_user_id(user_id).await?)
    }
}
