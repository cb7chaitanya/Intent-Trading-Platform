use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::cache::service::{CacheService, CacheTtl};
use crate::ledger::model::{EntryType, ReferenceType};
use crate::ledger::service::LedgerService;

use super::model::{Asset, Balance};
use super::repository::BalanceRepository;

#[derive(Debug)]
pub enum BalanceError {
    InsufficientBalance,
    InsufficientLockedBalance,
    InvalidAmount,
    DbError(sqlx::Error),
    LedgerError(String),
}

impl std::fmt::Display for BalanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BalanceError::InsufficientBalance => write!(f, "Insufficient available balance"),
            BalanceError::InsufficientLockedBalance => write!(f, "Insufficient locked balance"),
            BalanceError::InvalidAmount => write!(f, "Amount must be positive"),
            BalanceError::DbError(e) => write!(f, "Database error: {e}"),
            BalanceError::LedgerError(e) => write!(f, "Ledger error: {e}"),
        }
    }
}

impl From<sqlx::Error> for BalanceError {
    fn from(e: sqlx::Error) -> Self {
        BalanceError::DbError(e)
    }
}

pub struct BalanceService {
    repo: BalanceRepository,
    ledger: Arc<LedgerService>,
    cache: Option<Arc<CacheService>>,
}

impl BalanceService {
    pub fn new(repo: BalanceRepository, ledger: Arc<LedgerService>) -> Self {
        Self { repo, ledger, cache: None }
    }

    pub fn with_cache(mut self, cache: Arc<CacheService>) -> Self {
        self.cache = Some(cache);
        self
    }

    fn invalidate_cache(&self, account_id: Uuid) {
        if let Some(cache) = &self.cache {
            let cache = cache.clone();
            let key = account_id.to_string();
            tokio::spawn(async move {
                cache.invalidate("balances", &key).await;
            });
        }
    }

    pub async fn deposit(
        &self,
        account_id: Uuid,
        asset: Asset,
        amount: i64,
    ) -> Result<Balance, BalanceError> {
        if amount <= 0 { return Err(BalanceError::InvalidAmount); }

        let mut balance = self.repo.find_or_create(account_id, &asset).await?;
        balance.available_balance += amount;
        balance.updated_at = Utc::now();
        self.repo.update(&balance).await?;

        self.invalidate_cache(account_id);

        let reference_id = Uuid::new_v4();
        self.ledger
            .create_entry(account_id, asset, amount, EntryType::DEBIT, ReferenceType::DEPOSIT, reference_id)
            .await.map_err(|e| BalanceError::LedgerError(e.to_string()))?;

        Ok(balance)
    }

    pub async fn withdraw(
        &self,
        account_id: Uuid,
        asset: Asset,
        amount: i64,
    ) -> Result<Balance, BalanceError> {
        if amount <= 0 { return Err(BalanceError::InvalidAmount); }

        let mut balance = self.repo.find_or_create(account_id, &asset).await?;
        if balance.available_balance < amount { return Err(BalanceError::InsufficientBalance); }

        balance.available_balance -= amount;
        balance.updated_at = Utc::now();
        self.repo.update(&balance).await?;

        self.invalidate_cache(account_id);

        let reference_id = Uuid::new_v4();
        self.ledger
            .create_entry(account_id, asset, amount, EntryType::CREDIT, ReferenceType::WITHDRAWAL, reference_id)
            .await.map_err(|e| BalanceError::LedgerError(e.to_string()))?;

        Ok(balance)
    }

    pub async fn lock_balance(
        &self,
        account_id: Uuid,
        asset: Asset,
        amount: i64,
    ) -> Result<Balance, BalanceError> {
        if amount <= 0 { return Err(BalanceError::InvalidAmount); }

        let mut balance = self.repo.find_or_create(account_id, &asset).await?;
        if balance.available_balance < amount { return Err(BalanceError::InsufficientBalance); }

        balance.available_balance -= amount;
        balance.locked_balance += amount;
        balance.updated_at = Utc::now();
        self.repo.update(&balance).await?;

        self.invalidate_cache(account_id);

        Ok(balance)
    }

    pub async fn unlock_balance(
        &self,
        account_id: Uuid,
        asset: Asset,
        amount: i64,
    ) -> Result<Balance, BalanceError> {
        if amount <= 0 { return Err(BalanceError::InvalidAmount); }

        let mut balance = self.repo.find_or_create(account_id, &asset).await?;
        if balance.locked_balance < amount { return Err(BalanceError::InsufficientLockedBalance); }

        balance.locked_balance -= amount;
        balance.available_balance += amount;
        balance.updated_at = Utc::now();
        self.repo.update(&balance).await?;

        self.invalidate_cache(account_id);

        Ok(balance)
    }

    pub async fn transfer(
        &self,
        from_account_id: Uuid,
        to_account_id: Uuid,
        asset: Asset,
        amount: i64,
    ) -> Result<(Balance, Balance), BalanceError> {
        if amount <= 0 { return Err(BalanceError::InvalidAmount); }

        let mut from = self.repo.find_or_create(from_account_id, &asset).await?;
        if from.available_balance < amount { return Err(BalanceError::InsufficientBalance); }

        let mut to = self.repo.find_or_create(to_account_id, &asset).await?;

        let now = Utc::now();
        from.available_balance -= amount; from.updated_at = now;
        to.available_balance += amount; to.updated_at = now;

        self.repo.update(&from).await?;
        self.repo.update(&to).await?;

        self.invalidate_cache(from_account_id);
        self.invalidate_cache(to_account_id);

        let reference_id = Uuid::new_v4();
        self.ledger
            .create_double_entry(from_account_id, to_account_id, asset, amount, ReferenceType::TRADE, reference_id)
            .await.map_err(|e| BalanceError::LedgerError(e.to_string()))?;

        Ok((from, to))
    }

    pub async fn get_balances(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<Balance>, BalanceError> {
        let key = account_id.to_string();

        if let Some(cache) = &self.cache {
            if let Some(balances) = cache.get::<Vec<Balance>>("balances", &key).await {
                return Ok(balances);
            }
        }

        let balances = self.repo.find_by_account_id(account_id).await?;

        if let Some(cache) = &self.cache {
            cache.set("balances", &key, &balances, CacheTtl::BALANCES).await;
        }

        Ok(balances)
    }
}
