use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::balances::model::Asset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "entry_type", rename_all = "UPPERCASE")]
pub enum EntryType {
    DEBIT,
    CREDIT,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "reference_type", rename_all = "UPPERCASE")]
pub enum ReferenceType {
    TRADE,
    DEPOSIT,
    WITHDRAWAL,
    FEE,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub account_id: Uuid,
    pub asset: Asset,
    pub amount: i64,
    pub entry_type: EntryType,
    pub reference_type: ReferenceType,
    pub reference_id: Uuid,
    pub created_at: DateTime<Utc>,
}
