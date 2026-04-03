use chrono::Utc;
use uuid::Uuid;

use crate::balances::model::Asset;

use super::model::{EntryType, LedgerEntry, ReferenceType};
use super::repository::LedgerRepository;

#[derive(Debug)]
pub enum LedgerError {
    DbError(sqlx::Error),
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for LedgerError {
    fn from(e: sqlx::Error) -> Self {
        LedgerError::DbError(e)
    }
}

pub struct LedgerService {
    repo: LedgerRepository,
}

impl LedgerService {
    pub fn new(repo: LedgerRepository) -> Self {
        Self { repo }
    }

    pub async fn create_entry(
        &self,
        account_id: Uuid,
        asset: Asset,
        amount: i64,
        entry_type: EntryType,
        reference_type: ReferenceType,
        reference_id: Uuid,
    ) -> Result<LedgerEntry, LedgerError> {
        let entry = LedgerEntry {
            id: Uuid::new_v4(),
            account_id,
            asset,
            amount,
            entry_type,
            reference_type,
            reference_id,
            created_at: Utc::now(),
        };
        self.repo.insert(&entry).await?;
        Ok(entry)
    }

    pub async fn create_double_entry(
        &self,
        debit_account_id: Uuid,
        credit_account_id: Uuid,
        asset: Asset,
        amount: i64,
        reference_type: ReferenceType,
        reference_id: Uuid,
    ) -> Result<(LedgerEntry, LedgerEntry), LedgerError> {
        let debit = self
            .create_entry(
                debit_account_id,
                asset.clone(),
                amount,
                EntryType::DEBIT,
                reference_type.clone(),
                reference_id,
            )
            .await?;

        let credit = self
            .create_entry(
                credit_account_id,
                asset,
                amount,
                EntryType::CREDIT,
                reference_type,
                reference_id,
            )
            .await?;

        Ok((debit, credit))
    }

    pub async fn get_entries(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<LedgerEntry>, LedgerError> {
        Ok(self.repo.find_by_account_id(account_id).await?)
    }

    pub async fn get_entries_by_reference(
        &self,
        reference_id: Uuid,
    ) -> Result<Vec<LedgerEntry>, LedgerError> {
        Ok(self.repo.find_by_reference_id(reference_id).await?)
    }

    pub async fn get_balance(
        &self,
        account_id: Uuid,
        asset: Asset,
    ) -> Result<i64, LedgerError> {
        Ok(self.repo.compute_balance(account_id, &asset).await?)
    }
}
