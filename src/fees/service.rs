use chrono::Utc;
use uuid::Uuid;

use crate::balances::model::Asset;
use crate::ledger::model::{EntryType, LedgerEntry, ReferenceType};
use crate::markets::model::Market;
use crate::settlement::model::Trade;

const PLATFORM_ACCOUNT_ID: &str = "00000000-0000-0000-0000-000000000001";
const SOLVER_FEE_SHARE: f64 = 0.3; // solver gets 30% of total fee
const MAKER_SHARE: f64 = 0.4; // maker pays 40% of total fee
const TAKER_SHARE: f64 = 0.6; // taker pays 60% of total fee

#[derive(Debug, Clone, serde::Serialize)]
pub struct FeeBreakdown {
    pub total_fee: i64,
    pub platform_fee: i64,
    pub solver_fee: i64,
    pub maker_fee: i64,
    pub taker_fee: i64,
}

#[derive(Debug)]
pub enum FeeError {
    DbError(sqlx::Error),
}

impl std::fmt::Display for FeeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeeError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for FeeError {
    fn from(e: sqlx::Error) -> Self {
        FeeError::DbError(e)
    }
}

pub fn platform_account_id() -> Uuid {
    PLATFORM_ACCOUNT_ID.parse().unwrap()
}

/// Pure calculation — no side effects.
pub fn calculate_fees(trade: &Trade, market: &Market) -> FeeBreakdown {
    let total_fee = (trade.amount_in as f64 * market.fee_rate) as i64;
    let solver_fee = (total_fee as f64 * SOLVER_FEE_SHARE) as i64;
    let platform_fee = total_fee - solver_fee;
    let taker_fee = (total_fee as f64 * TAKER_SHARE) as i64;
    let maker_fee = total_fee - taker_fee;

    FeeBreakdown {
        total_fee,
        platform_fee,
        solver_fee,
        maker_fee,
        taker_fee,
    }
}

/// Apply fees inside an existing database transaction.
/// The caller owns the transaction and commits/rolls back.
pub async fn apply_fees(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trade: &Trade,
    fees: &FeeBreakdown,
) -> Result<(), FeeError> {
    let now = Utc::now();
    let platform_id = platform_account_id();

    // Debit taker fee from buyer
    debit_fee(tx, trade.buyer_account_id, &trade.asset_in, fees.taker_fee, now).await?;

    // Debit maker fee from seller
    debit_fee(tx, trade.seller_account_id, &trade.asset_in, fees.maker_fee, now).await?;

    // Credit platform
    credit_fee(tx, platform_id, &trade.asset_in, fees.platform_fee, now).await?;

    // Credit solver
    credit_fee(tx, trade.solver_account_id, &trade.asset_in, fees.solver_fee, now).await?;

    // Ledger entries — taker fee
    insert_fee_ledger(
        tx, trade.buyer_account_id, &trade.asset_in, fees.taker_fee,
        EntryType::CREDIT, trade.id, now,
    ).await?;

    // Ledger entries — maker fee
    insert_fee_ledger(
        tx, trade.seller_account_id, &trade.asset_in, fees.maker_fee,
        EntryType::CREDIT, trade.id, now,
    ).await?;

    // Ledger entries — platform receives fee
    insert_fee_ledger(
        tx, platform_id, &trade.asset_in, fees.platform_fee,
        EntryType::DEBIT, trade.id, now,
    ).await?;

    // Ledger entries — solver receives fee
    insert_fee_ledger(
        tx, trade.solver_account_id, &trade.asset_in, fees.solver_fee,
        EntryType::DEBIT, trade.id, now,
    ).await?;

    Ok(())
}

async fn debit_fee(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    if amount == 0 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE balances SET available_balance = available_balance - $1, updated_at = $2
         WHERE account_id = $3 AND asset = $4",
    )
    .bind(amount)
    .bind(now)
    .bind(account_id)
    .bind(asset)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn credit_fee(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    if amount == 0 {
        return Ok(());
    }
    // Ensure balance row exists
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM balances WHERE account_id = $1 AND asset = $2)",
    )
    .bind(account_id)
    .bind(asset)
    .fetch_one(&mut **tx)
    .await?;

    if !exists {
        sqlx::query(
            "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
             VALUES ($1, $2, $3, $4, 0, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(account_id)
        .bind(asset)
        .bind(amount)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE balances SET available_balance = available_balance + $1, updated_at = $2
             WHERE account_id = $3 AND asset = $4",
        )
        .bind(amount)
        .bind(now)
        .bind(account_id)
        .bind(asset)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_fee_ledger(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    entry_type: EntryType,
    reference_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    if amount == 0 {
        return Ok(());
    }
    let entry = LedgerEntry {
        id: Uuid::new_v4(),
        account_id,
        asset: asset.clone(),
        amount,
        entry_type,
        reference_type: ReferenceType::FEE,
        reference_id,
        created_at: now,
    };
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}
