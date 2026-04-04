use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::balances::model::{Asset, Balance};
use crate::fees::service as fee_engine;
use crate::ledger::model::{EntryType, LedgerEntry, ReferenceType};
use crate::markets::model::Market;
use crate::metrics::{counters, histograms};
use crate::models::fill::Fill;
use crate::models::intent::IntentStatus;

use super::model::{CreateTradeRequest, Trade, TradeStatus};

const PLATFORM_ACCOUNT_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Debug)]
pub enum SettlementError {
    TradeNotFound,
    FillNotFound,
    AlreadySettled,
    InsufficientBalance,
    FeeError(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for SettlementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettlementError::TradeNotFound => write!(f, "Trade not found"),
            SettlementError::FillNotFound => write!(f, "Fill not found"),
            SettlementError::AlreadySettled => write!(f, "Already settled"),
            SettlementError::InsufficientBalance => write!(f, "Insufficient balance"),
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

    pub fn platform_account_id() -> Uuid {
        PLATFORM_ACCOUNT_ID.parse().unwrap()
    }

    // ---------------------------------------------------------------
    // Trade CRUD
    // ---------------------------------------------------------------

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
        .bind(trade.id).bind(trade.buyer_account_id).bind(trade.seller_account_id)
        .bind(trade.solver_account_id).bind(&trade.asset_in).bind(&trade.asset_out)
        .bind(trade.amount_in).bind(trade.amount_out).bind(trade.platform_fee)
        .bind(trade.solver_fee).bind(&trade.status).bind(trade.created_at).bind(trade.settled_at)
        .execute(&self.pool).await?;

        Ok(trade)
    }

    pub async fn get_trade(&self, trade_id: Uuid) -> Result<Option<Trade>, SettlementError> {
        Ok(sqlx::query_as::<_, Trade>("SELECT * FROM trades WHERE id = $1")
            .bind(trade_id).fetch_optional(&self.pool).await?)
    }

    // ---------------------------------------------------------------
    // Per-fill settlement
    // ---------------------------------------------------------------

    /// Settle a single fill atomically in one transaction.
    pub async fn settle_fill(
        &self,
        fill_id: Uuid,
        buyer_account_id: Uuid,
        seller_account_id: Uuid,
        asset_in: &Asset,
        asset_out: &Asset,
        fee_rate: f64,
    ) -> Result<(), SettlementError> {
        let settle_start = std::time::Instant::now();
        let mut tx = self.pool.begin().await?;

        // Lock the fill row
        let fill = sqlx::query_as::<_, Fill>(
            "SELECT * FROM fills WHERE id = $1 FOR UPDATE",
        )
        .bind(fill_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SettlementError::FillNotFound)?;

        if fill.settled {
            return Err(SettlementError::AlreadySettled);
        }

        let now = Utc::now();
        let platform_id = Self::platform_account_id();
        let amount = fill.filled_qty;

        // Fees for this fill
        let total_fee = (amount as f64 * fee_rate) as i64;
        let solver_fee = (total_fee as f64 * 0.3) as i64;
        let platform_fee = total_fee - solver_fee;
        let seller_receives = amount - total_fee;

        tracing::info!(
            fill_id = %fill_id, intent_id = %fill.intent_id,
            solver_id = %fill.solver_id, amount, fee = total_fee,
            "settle_fill_started"
        );

        // 1. Unlock buyer's locked funds and consume them
        unlock_and_debit(&mut tx, buyer_account_id, asset_in, amount, now).await?;

        // 2. Credit seller (minus fees)
        credit_balance(&mut tx, seller_account_id, asset_in, seller_receives, now).await?;

        // 3. Credit buyer with received asset
        credit_balance(&mut tx, buyer_account_id, asset_out, fill.qty, now).await?;

        // 4. Debit seller's outgoing asset
        debit_balance(&mut tx, seller_account_id, asset_out, fill.qty, now)
            .await.map_err(|_| SettlementError::InsufficientBalance)?;

        // 5. Platform fee
        if platform_fee > 0 {
            credit_balance(&mut tx, platform_id, asset_in, platform_fee, now).await?;
        }

        // 6. Solver fee — look up solver's account
        if solver_fee > 0 {
            let solver_acc = sqlx::query_scalar::<_, Uuid>(
                "SELECT a.id FROM accounts a JOIN users u ON u.id = a.user_id
                 WHERE u.id::text = $1 LIMIT 1",
            )
            .bind(&fill.solver_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(platform_id);

            credit_balance(&mut tx, solver_acc, asset_in, solver_fee, now).await?;
        }

        // Ledger entries
        insert_ledger(&mut tx, buyer_account_id, asset_in, amount,
            EntryType::CREDIT, ReferenceType::TRADE, fill.id, now).await?;
        insert_ledger(&mut tx, buyer_account_id, asset_out, fill.qty,
            EntryType::DEBIT, ReferenceType::TRADE, fill.id, now).await?;
        insert_ledger(&mut tx, seller_account_id, asset_in, seller_receives,
            EntryType::DEBIT, ReferenceType::TRADE, fill.id, now).await?;
        insert_ledger(&mut tx, seller_account_id, asset_out, fill.qty,
            EntryType::CREDIT, ReferenceType::TRADE, fill.id, now).await?;
        if platform_fee > 0 {
            insert_ledger(&mut tx, platform_id, asset_in, platform_fee,
                EntryType::DEBIT, ReferenceType::FEE, fill.id, now).await?;
        }
        if solver_fee > 0 {
            insert_ledger(&mut tx, platform_id, asset_in, solver_fee,
                EntryType::DEBIT, ReferenceType::FEE, fill.id, now).await?;
        }

        // Mark fill settled
        sqlx::query("UPDATE fills SET settled = TRUE, settled_at = $1 WHERE id = $2")
            .bind(now).bind(fill_id)
            .execute(&mut *tx).await?;

        tx.commit().await?;

        let duration_ms = settle_start.elapsed().as_secs_f64() * 1000.0;
        counters::SETTLEMENT_SUCCESS_TOTAL.inc();
        counters::TRADES_TOTAL.inc();
        histograms::SETTLEMENT_DURATION.observe(settle_start.elapsed().as_secs_f64());

        tracing::info!(fill_id = %fill_id, intent_id = %fill.intent_id, duration_ms, "settle_fill_success");
        Ok(())
    }

    /// Settle all unsettled fills for an intent, then update intent status.
    pub async fn settle_intent_fills(
        &self,
        intent_id: Uuid,
        buyer_account_id: Uuid,
        seller_account_id: Uuid,
        asset_in: &Asset,
        asset_out: &Asset,
        fee_rate: f64,
    ) -> Result<IntentStatus, SettlementError> {
        let unsettled = sqlx::query_as::<_, Fill>(
            "SELECT * FROM fills WHERE intent_id = $1 AND settled = FALSE ORDER BY price DESC",
        )
        .bind(intent_id).fetch_all(&self.pool).await?;

        for fill in &unsettled {
            match self.settle_fill(fill.id, buyer_account_id, seller_account_id, asset_in, asset_out, fee_rate).await {
                Ok(()) | Err(SettlementError::AlreadySettled) => {}
                Err(e) => {
                    let _ = super::retry::record_fill_failure(&self.pool, fill.id, &e.to_string()).await;
                    tracing::error!(fill_id = %fill.id, intent_id = %intent_id, error = %e, "settle_fill_failed");
                }
            }
        }

        let status = self.compute_intent_status(intent_id).await?;
        sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
            .bind(&status).bind(intent_id)
            .execute(&self.pool).await?;

        tracing::info!(intent_id = %intent_id, status = ?status, "intent_status_updated");
        Ok(status)
    }

    async fn compute_intent_status(&self, intent_id: Uuid) -> Result<IntentStatus, SettlementError> {
        let intent_amount = sqlx::query_scalar::<_, i64>(
            "SELECT amount_in FROM intents WHERE id = $1",
        )
        .bind(intent_id).fetch_one(&self.pool).await?;

        let settled_qty = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(filled_qty), 0) FROM fills WHERE intent_id = $1 AND settled = TRUE",
        )
        .bind(intent_id).fetch_one(&self.pool).await?;

        let total_fills = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fills WHERE intent_id = $1",
        )
        .bind(intent_id).fetch_one(&self.pool).await?;

        if total_fills == 0 {
            return Ok(IntentStatus::Failed);
        }
        if settled_qty >= intent_amount {
            Ok(IntentStatus::Completed)
        } else if settled_qty > 0 {
            Ok(IntentStatus::PartiallyFilled)
        } else {
            Ok(IntentStatus::Executing)
        }
    }

    // ---------------------------------------------------------------
    // Legacy single-trade settlement
    // ---------------------------------------------------------------

    pub async fn settle_trade(&self, trade_id: Uuid) -> Result<Trade, SettlementError> {
        tracing::info!(trade_id = %trade_id, "settlement_started");
        let settle_start = std::time::Instant::now();
        let mut tx = self.pool.begin().await?;

        let trade = sqlx::query_as::<_, Trade>("SELECT * FROM trades WHERE id = $1 FOR UPDATE")
            .bind(trade_id).fetch_optional(&mut *tx).await?
            .ok_or(SettlementError::TradeNotFound)?;

        if trade.status == TradeStatus::Settled {
            return Err(SettlementError::AlreadySettled);
        }

        let now = Utc::now();
        let platform_id = Self::platform_account_id();

        debit_balance(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in, now)
            .await.map_err(|_| SettlementError::InsufficientBalance)?;
        let seller_receives = trade.amount_in - trade.platform_fee - trade.solver_fee;
        credit_balance(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives, now).await?;
        credit_balance(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out, now).await?;
        debit_balance(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out, now)
            .await.map_err(|_| SettlementError::InsufficientBalance)?;
        credit_balance(&mut tx, platform_id, &trade.asset_in, trade.platform_fee, now).await?;
        credit_balance(&mut tx, trade.solver_account_id, &trade.asset_in, trade.solver_fee, now).await?;

        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_in, trade.amount_in,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.buyer_account_id, &trade.asset_out, trade.amount_out,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_in, seller_receives,
            EntryType::DEBIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.seller_account_id, &trade.asset_out, trade.amount_out,
            EntryType::CREDIT, ReferenceType::TRADE, trade.id, now).await?;
        insert_ledger(&mut tx, platform_id, &trade.asset_in, trade.platform_fee,
            EntryType::DEBIT, ReferenceType::FEE, trade.id, now).await?;
        insert_ledger(&mut tx, trade.solver_account_id, &trade.asset_in, trade.solver_fee,
            EntryType::DEBIT, ReferenceType::FEE, trade.id, now).await?;

        sqlx::query("UPDATE trades SET status = $1, settled_at = $2 WHERE id = $3")
            .bind(TradeStatus::Settled).bind(now).bind(trade.id)
            .execute(&mut *tx).await?;

        tx.commit().await?;

        counters::SETTLEMENT_SUCCESS_TOTAL.inc();
        histograms::SETTLEMENT_DURATION.observe(settle_start.elapsed().as_secs_f64());
        tracing::info!(trade_id = %trade_id, "settlement_success");

        Ok(Trade { status: TradeStatus::Settled, settled_at: Some(now), ..trade })
    }

    pub async fn settle_trade_with_retry(&self, trade_id: Uuid) -> Result<Trade, SettlementError> {
        match self.settle_trade(trade_id).await {
            Ok(trade) => Ok(trade),
            Err(SettlementError::AlreadySettled) => Err(SettlementError::AlreadySettled),
            Err(e) => {
                let _ = super::retry::record_failure(&self.pool, trade_id, &e.to_string()).await;
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------
// Transaction helpers
// ---------------------------------------------------------------

async fn unlock_and_debit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid, asset: &Asset, amount: i64, now: chrono::DateTime<Utc>,
) -> Result<(), SettlementError> {
    let result = sqlx::query(
        "UPDATE balances SET locked_balance = locked_balance - $1, updated_at = $2
         WHERE account_id = $3 AND asset = $4 AND locked_balance >= $1",
    )
    .bind(amount).bind(now).bind(account_id).bind(asset)
    .execute(&mut **tx).await.map_err(SettlementError::DbError)?;

    if result.rows_affected() == 0 {
        return Err(SettlementError::InsufficientBalance);
    }
    Ok(())
}

async fn find_or_create_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid, asset: &Asset, now: chrono::DateTime<Utc>,
) -> Result<Balance, sqlx::Error> {
    if let Some(b) = sqlx::query_as::<_, Balance>(
        "SELECT * FROM balances WHERE account_id = $1 AND asset = $2 FOR UPDATE",
    ).bind(account_id).bind(asset).fetch_optional(&mut **tx).await? {
        return Ok(b);
    }
    let b = Balance { id: Uuid::new_v4(), account_id, asset: asset.clone(),
        available_balance: 0, locked_balance: 0, updated_at: now };
    sqlx::query(
        "INSERT INTO balances (id, account_id, asset, available_balance, locked_balance, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    ).bind(b.id).bind(b.account_id).bind(&b.asset).bind(b.available_balance)
    .bind(b.locked_balance).bind(b.updated_at).execute(&mut **tx).await?;
    Ok(b)
}

async fn debit_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid, asset: &Asset, amount: i64, now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut b = find_or_create_balance(tx, account_id, asset, now).await?;
    if b.available_balance < amount { return Err(sqlx::Error::Protocol("Insufficient balance".into())); }
    b.available_balance -= amount; b.updated_at = now;
    sqlx::query("UPDATE balances SET available_balance = $1, updated_at = $2 WHERE id = $3")
        .bind(b.available_balance).bind(b.updated_at).bind(b.id).execute(&mut **tx).await?;
    Ok(())
}

async fn credit_balance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid, asset: &Asset, amount: i64, now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut b = find_or_create_balance(tx, account_id, asset, now).await?;
    b.available_balance += amount; b.updated_at = now;
    sqlx::query("UPDATE balances SET available_balance = $1, updated_at = $2 WHERE id = $3")
        .bind(b.available_balance).bind(b.updated_at).bind(b.id).execute(&mut **tx).await?;
    Ok(())
}

async fn insert_ledger(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid, asset: &Asset, amount: i64,
    entry_type: EntryType, reference_type: ReferenceType, reference_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let e = LedgerEntry { id: Uuid::new_v4(), account_id, asset: asset.clone(), amount,
        entry_type, reference_type, reference_id, created_at: now };
    sqlx::query(
        "INSERT INTO ledger_entries (id, account_id, asset, amount, entry_type, reference_type, reference_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    ).bind(e.id).bind(e.account_id).bind(&e.asset).bind(e.amount)
    .bind(&e.entry_type).bind(&e.reference_type).bind(e.reference_id).bind(e.created_at)
    .execute(&mut **tx).await?;
    Ok(())
}
