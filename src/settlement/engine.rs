use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::balances::model::{Asset, Balance};
use crate::fees::service as fee_engine;
use crate::ledger::model::{EntryType, LedgerEntry, ReferenceType};
use crate::markets::model::Market;
use crate::metrics::{counters, histograms};

use super::model::{CreateTradeRequest, Trade, TradeStatus};

const PLATFORM_ACCOUNT_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Debug)]
pub enum SettlementError {
    TradeNotFound,
    AlreadySettled,
    InsufficientBalance,
    FeeError(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for SettlementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettlementError::TradeNotFound => write!(f, "Trade not found"),
            SettlementError::AlreadySettled => write!(f, "Trade already settled"),
            SettlementError::InsufficientBalance => write!(f, "Insufficient balance for settlement"),
            SettlementError::FeeError(e) => write!(f, "Fee error: {e}"),
            SettlementError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for SettlementError {
    fn from(e: sqlx::Error) -> Self {
        SettlementError::DbError(e)
    }
}

pub struct SettlementEngine {
    pool: PgPool,
}

impl SettlementEngine {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Settle with automatic failure recording for retry.
    pub async fn settle_trade_with_retry(&self, trade_id: Uuid) -> Result<Trade, SettlementError> {
        match self.settle_trade(trade_id).await {
            Ok(trade) => Ok(trade),
            Err(SettlementError::AlreadySettled) => {
                // Idempotent — not a failure
                Err(SettlementError::AlreadySettled)
            }
            Err(e) => {
                // Record for retry
                let _ = super::retry::record_failure(&self.pool, trade_id, &e.to_string()).await;
                Err(e)
            }
        }
    }

    pub fn platform_account_id() -> Uuid {
        PLATFORM_ACCOUNT_ID.parse().unwrap()
    }

    pub async fn create_trade(&self, req: CreateTradeRequest) -> Result<Trade, SettlementError> {
        let now = Utc::now();
        let trade = Trade {
            id: Uuid::new_v4(),
            buyer_account_id: req.buyer_account_id,
            seller_account_id: req.seller_account_id,
            solver_account_id: req.solver_account_id,
            asset_in: req.asset_in,
            asset_out: req.asset_out,
            amount_in: req.amount_in,
            amount_out: req.amount_out,
            platform_fee: req.platform_fee,
            solver_fee: req.solver_fee,
            status: TradeStatus::Pending,
            created_at: now,
            settled_at: None,
        };

        sqlx::query(
            "INSERT INTO trades (id, buyer_account_id, seller_account_id, solver_account_id,
                asset_in, asset_out, amount_in, amount_out, platform_fee, solver_fee,
                status, created_at, settled_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(trade.id)
        .bind(trade.buyer_account_id)
        .bind(trade.seller_account_id)
        .bind(trade.solver_account_id)
        .bind(&trade.asset_in)
        .bind(&trade.asset_out)
        .bind(trade.amount_in)
        .bind(trade.amount_out)
        .bind(trade.platform_fee)
        .bind(trade.solver_fee)
        .bind(&trade.status)
        .bind(trade.created_at)
        .bind(trade.settled_at)
        .execute(&self.pool)
        .await?;

        Ok(trade)
    }

    pub async fn settle_trade(&self, trade_id: Uuid) -> Result<Trade, SettlementError> {
        tracing::info!(trade_id = %trade_id, "settlement_started");
        let settle_start = std::time::Instant::now();
        let mut tx = self.pool.begin().await?;

        // Fetch trade inside transaction
        let trade = sqlx::query_as::<_, Trade>("SELECT * FROM trades WHERE id = $1 FOR UPDATE")
            .bind(trade_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(SettlementError::TradeNotFound)?;

        if trade.status == TradeStatus::Settled {
            return Err(SettlementError::AlreadySettled);
        }

        let now = Utc::now();
        let platform_account_id = Self::platform_account_id();

        // 1. Debit buyer: subtract asset_in from buyer
        debit_balance(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in, now)
            .await
            .map_err(|_| SettlementError::InsufficientBalance)?;

        // 2. Credit seller: add asset_in to seller (minus fees)
        let seller_receives = trade.amount_in - trade.platform_fee - trade.solver_fee;
        credit_balance(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives, now).await?;

        // 3. Credit buyer: add asset_out to buyer
        credit_balance(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out, now).await?;

        // 4. Debit seller: subtract asset_out from seller
        debit_balance(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out, now)
            .await
            .map_err(|_| SettlementError::InsufficientBalance)?;

        // 5. Platform fee
        credit_balance(&mut tx, platform_account_id, &trade.asset_in, trade.platform_fee, now).await?;

        // 6. Solver fee
        credit_balance(&mut tx, trade.solver_account_id, &trade.asset_in, trade.solver_fee, now).await?;

        // Ledger entries — buyer side
        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;

        // Ledger entries — seller side
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;

        // Ledger entries — platform fee
        insert_ledger(&mut tx, platform_account_id, &trade.asset_in, trade.platform_fee,
            EntryType::DEBIT, ReferenceType::FEE, trade.id, now).await?;

        // Ledger entries — solver fee
        insert_ledger(&mut tx, trade.solver_account_id, &trade.asset_in, trade.solver_fee,
            EntryType::DEBIT, ReferenceType::FEE, trade.id, now).await?;

        // Mark trade as settled
        sqlx::query("UPDATE trades SET status = $1, settled_at = $2 WHERE id = $3")
            .bind(TradeStatus::Settled)
            .bind(now)
            .bind(trade.id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        let duration_ms = settle_start.elapsed().as_secs_f64() * 1000.0;
        counters::SETTLEMENT_SUCCESS_TOTAL.inc();
        histograms::SETTLEMENT_DURATION.observe(settle_start.elapsed().as_secs_f64());

        tracing::info!(
            trade_id = %trade_id,
            duration_ms = duration_ms,
            "settlement_success"
        );

        Ok(Trade {
            status: TradeStatus::Settled,
            settled_at: Some(now),
            ..trade
        })
    }

    /// Settle a trade using market-driven fee calculation.
    /// Fees are computed from the market's fee_rate and applied atomically.
    pub async fn settle_trade_with_market(
        &self,
        trade_id: Uuid,
        market: &Market,
    ) -> Result<(Trade, fee_engine::FeeBreakdown), SettlementError> {
        let mut tx = self.pool.begin().await?;

        let trade = sqlx::query_as::<_, Trade>("SELECT * FROM trades WHERE id = $1 FOR UPDATE")
            .bind(trade_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(SettlementError::TradeNotFound)?;

        if trade.status == TradeStatus::Settled {
            return Err(SettlementError::AlreadySettled);
        }

        let now = Utc::now();

        // Calculate fees from market config
        let fees = fee_engine::calculate_fees(&trade, market);

        // Asset swaps (excluding fees — fee engine handles those)
        let seller_receives = trade.amount_in - fees.total_fee;

        debit_balance(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in, now)
            .await
            .map_err(|_| SettlementError::InsufficientBalance)?;

        credit_balance(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives, now).await?;

        credit_balance(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out, now).await?;

        debit_balance(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out, now)
            .await
            .map_err(|_| SettlementError::InsufficientBalance)?;

        // Trade ledger entries
        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;

        // Apply fees atomically in the same transaction
        fee_engine::apply_fees(&mut tx, &trade, &fees)
            .await
            .map_err(|e| SettlementError::FeeError(e.to_string()))?;

        // Update trade with calculated fees and mark settled
        sqlx::query(
            "UPDATE trades SET platform_fee = $1, solver_fee = $2, status = $3, settled_at = $4 WHERE id = $5",
        )
        .bind(fees.platform_fee)
        .bind(fees.solver_fee)
        .bind(TradeStatus::Settled)
        .bind(now)
        .bind(trade.id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        let settled = Trade {
            platform_fee: fees.platform_fee,
            solver_fee: fees.solver_fee,
            status: TradeStatus::Settled,
            settled_at: Some(now),
            ..trade
        };

        Ok((settled, fees))
    }

    pub async fn get_trade(&self, trade_id: Uuid) -> Result<Option<Trade>, SettlementError> {
        Ok(
            sqlx::query_as::<_, Trade>("SELECT * FROM trades WHERE id = $1")
                .bind(trade_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}

async fn find_or_create_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    now: chrono::DateTime<Utc>,
) -> Result<Balance, sqlx::Error> {
    let existing = sqlx::query_as::<_, Balance>(
        "SELECT * FROM balances WHERE account_id = $1 AND asset = $2 FOR UPDATE",
    )
    .bind(account_id)
    .bind(asset)
    .fetch_optional(&mut **tx)
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
        updated_at: now,
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
    .execute(&mut **tx)
    .await?;

    Ok(balance)
}

async fn debit_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut balance = find_or_create_balance(tx, account_id, asset, now).await?;
    if balance.available_balance < amount {
        return Err(sqlx::Error::Protocol("Insufficient balance".to_string()));
    }
    balance.available_balance -= amount;
    balance.updated_at = now;
    sqlx::query(
        "UPDATE balances SET available_balance = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(balance.available_balance)
    .bind(balance.updated_at)
    .bind(balance.id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn credit_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut balance = find_or_create_balance(tx, account_id, asset, now).await?;
    balance.available_balance += amount;
    balance.updated_at = now;
    sqlx::query(
        "UPDATE balances SET available_balance = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(balance.available_balance)
    .bind(balance.updated_at)
    .bind(balance.id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_ledger(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    asset: &Asset,
    amount: i64,
    entry_type: EntryType,
    reference_type: ReferenceType,
    reference_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let entry = LedgerEntry {
        id: Uuid::new_v4(),
        account_id,
        asset: asset.clone(),
        amount,
        entry_type,
        reference_type,
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
