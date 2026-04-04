use std::sync::Arc;

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::metrics::counters;
use crate::balances::model::Asset;
use crate::db::redis::{Event, EventBus};
use crate::db::storage::Storage;
use crate::db::stream_bus::{StreamBus, STREAM_INTENT_CREATED};
use crate::models::intent::{Intent, IntentStatus};
use crate::risk::service::{IntentRiskParams, RiskEngine};

#[derive(Debug)]
pub enum IntentError {
    InsufficientBalance,
    InvalidAsset(String),
    IntentNotFound,
    NotAmendable(String),
    RiskRejected(String),
    RedisError(redis::RedisError),
    BalanceError(String),
    StorageError(String),
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentError::InsufficientBalance => write!(f, "Insufficient balance"),
            IntentError::InvalidAsset(a) => write!(f, "Invalid asset: {a}"),
            IntentError::IntentNotFound => write!(f, "Intent not found"),
            IntentError::NotAmendable(s) => write!(f, "Intent cannot be amended: {s}"),
            IntentError::RedisError(e) => write!(f, "Redis error: {e}"),
            IntentError::RiskRejected(e) => write!(f, "Risk rejected: {e}"),
            IntentError::BalanceError(e) => write!(f, "Balance error: {e}"),
            IntentError::StorageError(e) => write!(f, "Storage error: {e}"),
        }
    }
}

impl From<redis::RedisError> for IntentError {
    fn from(e: redis::RedisError) -> Self {
        IntentError::RedisError(e)
    }
}

pub struct IntentService {
    storage: Arc<Storage>,
    event_bus: EventBus,
    stream_bus: Arc<StreamBus>,
    risk_engine: Arc<RiskEngine>,
}

impl IntentService {
    pub fn new(
        storage: Arc<Storage>,
        event_bus: EventBus,
        stream_bus: Arc<StreamBus>,
        risk_engine: Arc<RiskEngine>,
    ) -> Self {
        Self {
            storage,
            event_bus,
            stream_bus,
            risk_engine,
        }
    }

    pub async fn create_intent(
        &mut self,
        user_id: String,
        account_id: Uuid,
        token_in: String,
        token_out: String,
        amount_in: u64,
        min_amount_out: u64,
        deadline: i64,
    ) -> Result<Intent, IntentError> {
        // Pre-transaction risk checks (rate limit, daily volume, market exists)
        let risk_params = IntentRiskParams {
            user_id: user_id.clone(),
            account_id,
            token_in: token_in.clone(),
            token_out: token_out.clone(),
            amount_in,
            min_amount_out,
        };
        self.risk_engine
            .validate_intent(&risk_params)
            .await
            .map_err(|e| IntentError::RiskRejected(e.to_string()))?;

        let asset = parse_asset(&token_in)?;
        let amount = amount_in as i64;

        // Atomic transaction: check balance → lock funds → insert intent
        let intent = Intent::new(
            user_id.clone(), token_in, token_out, amount_in, min_amount_out, deadline,
        );

        let pool = self.storage.pool();
        let mut tx = pool.begin().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        // 1. SELECT balance FOR UPDATE (row lock prevents concurrent modification)
        let row = sqlx::query(
            "SELECT available_balance FROM balances
             WHERE account_id = $1 AND asset = $2
             FOR UPDATE",
        )
        .bind(account_id)
        .bind(&asset)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        let available: i64 = match row {
            Some(r) => r.get("available_balance"),
            None => 0,
        };

        // 2. Check sufficient balance
        if available < amount {
            // Transaction rolls back on drop
            return Err(IntentError::InsufficientBalance);
        }

        // 3. Lock balance (move from available to locked)
        let now = Utc::now();
        sqlx::query(
            "UPDATE balances
             SET available_balance = available_balance - $1,
                 locked_balance = locked_balance + $1,
                 updated_at = $2
             WHERE account_id = $3 AND asset = $4",
        )
        .bind(amount)
        .bind(now)
        .bind(account_id)
        .bind(&asset)
        .execute(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        // 4. Insert intent
        sqlx::query(
            "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out, deadline, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(intent.id)
        .bind(&intent.user_id)
        .bind(&intent.token_in)
        .bind(&intent.token_out)
        .bind(intent.amount_in)
        .bind(intent.min_amount_out)
        .bind(intent.deadline)
        .bind(&intent.status)
        .bind(intent.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        // 5. COMMIT
        tx.commit()
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;

        // Post-transaction: update risk counters, publish events
        self.risk_engine
            .record_accepted_intent(&user_id, amount_in)
            .await;

        self.event_bus
            .publish(&Event::IntentCreated(intent.clone()))
            .await?;

        let _ = self
            .stream_bus
            .publish(STREAM_INTENT_CREATED, &intent)
            .await;

        counters::INTENTS_TOTAL.inc();

        tracing::info!(
            intent_id = %intent.id,
            user_id = %intent.user_id,
            token_in = %intent.token_in,
            token_out = %intent.token_out,
            amount_in = intent.amount_in,
            "intent_created"
        );

        Ok(intent)
    }

    pub async fn get_intent(&self, intent_id: &Uuid) -> Option<Intent> {
        self.storage.get_intent(intent_id).await
    }

    pub async fn list_intents(&self) -> Vec<Intent> {
        self.storage.list_intents().await
    }

    pub async fn update_intent_status(
        &self,
        intent_id: &Uuid,
        status: IntentStatus,
    ) -> Option<Intent> {
        let mut intent = self.storage.get_intent(intent_id).await?;
        intent.status = status;
        let _ = self.storage.update_intent(&intent).await;
        Some(intent)
    }

    pub async fn cancel_intent(
        &mut self,
        intent_id: &Uuid,
        account_id: Uuid,
    ) -> Result<Option<Intent>, IntentError> {
        let Some(mut intent) = self.storage.get_intent(intent_id).await else {
            return Ok(None);
        };

        let asset = parse_asset(&intent.token_in)?;
        let amount = intent.amount_in;

        // Atomic: unlock balance + update intent status
        let pool = self.storage.pool();
        let mut tx = pool.begin().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        sqlx::query(
            "UPDATE balances
             SET available_balance = available_balance + $1,
                 locked_balance = locked_balance - $1,
                 updated_at = NOW()
             WHERE account_id = $2 AND asset = $3",
        )
        .bind(amount)
        .bind(account_id)
        .bind(&asset)
        .execute(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
            .bind(IntentStatus::Cancelled)
            .bind(intent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;

        tx.commit().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        intent.status = IntentStatus::Cancelled;
        self.event_bus
            .publish(&Event::IntentCancelled(intent.clone()))
            .await?;
        Ok(Some(intent))
    }

    pub async fn fail_intent(
        &mut self,
        intent_id: &Uuid,
        account_id: Uuid,
    ) -> Result<Option<Intent>, IntentError> {
        let Some(mut intent) = self.storage.get_intent(intent_id).await else {
            return Ok(None);
        };

        let asset = parse_asset(&intent.token_in)?;
        let amount = intent.amount_in;

        // Atomic: unlock balance + update intent status
        let pool = self.storage.pool();
        let mut tx = pool.begin().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        sqlx::query(
            "UPDATE balances
             SET available_balance = available_balance + $1,
                 locked_balance = locked_balance - $1,
                 updated_at = NOW()
             WHERE account_id = $2 AND asset = $3",
        )
        .bind(amount)
        .bind(account_id)
        .bind(&asset)
        .execute(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
            .bind(IntentStatus::Failed)
            .bind(intent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;

        tx.commit().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        intent.status = IntentStatus::Failed;
        self.event_bus
            .publish(&Event::IntentFailed(intent.clone()))
            .await?;
        Ok(Some(intent))
    }

    pub async fn start_bidding(
        &mut self,
        intent_id: &Uuid,
    ) -> Result<Option<Intent>, IntentError> {
        let Some(mut intent) = self.storage.get_intent(intent_id).await else {
            return Ok(None);
        };
        intent.status = IntentStatus::Bidding;
        let _ = self.storage.update_intent(&intent).await;
        self.event_bus
            .publish(&Event::IntentBidding(intent.clone()))
            .await?;
        Ok(Some(intent))
    }

    /// Amend an intent's amount or price before it's matched.
    /// Atomic: locks intent row, adjusts balance, records amendment, resets to Open.
    pub async fn amend_intent(
        &mut self,
        intent_id: &Uuid,
        account_id: Uuid,
        new_amount_in: Option<u64>,
        new_min_amount_out: Option<u64>,
        new_limit_price: Option<i64>,
    ) -> Result<Intent, IntentError> {
        let pool = self.storage.pool();
        let mut tx = pool.begin().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        // Lock intent row
        let intent = sqlx::query_as::<_, Intent>(
            "SELECT * FROM intents WHERE id = $1 FOR UPDATE",
        )
        .bind(intent_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?
        .ok_or(IntentError::IntentNotFound)?;

        // Only Open or Bidding intents can be amended
        if intent.status != IntentStatus::Open && intent.status != IntentStatus::Bidding {
            return Err(IntentError::NotAmendable(format!("status is {:?}", intent.status)));
        }

        let asset = parse_asset(&intent.token_in)?;
        let old_amount = intent.amount_in;
        let new_amount = new_amount_in.map(|a| a as i64).unwrap_or(old_amount);
        let now = Utc::now();

        // Get amendment count for numbering
        let amendment_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM intent_amendments WHERE intent_id = $1",
        )
        .bind(intent_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))? as i32;

        let amendment_num = amendment_count + 1;

        // Balance adjustment
        let diff = new_amount - old_amount;
        if diff > 0 {
            // Need to lock more — check available balance
            let available: i64 = sqlx::query_scalar(
                "SELECT available_balance FROM balances WHERE account_id = $1 AND asset = $2 FOR UPDATE",
            )
            .bind(account_id)
            .bind(&asset)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?
            .unwrap_or(0);

            if available < diff {
                return Err(IntentError::InsufficientBalance);
            }

            sqlx::query(
                "UPDATE balances SET available_balance = available_balance - $1,
                     locked_balance = locked_balance + $1, updated_at = $2
                 WHERE account_id = $3 AND asset = $4",
            )
            .bind(diff).bind(now).bind(account_id).bind(&asset)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;
        } else if diff < 0 {
            // Unlock excess
            sqlx::query(
                "UPDATE balances SET available_balance = available_balance + $1,
                     locked_balance = locked_balance - $1, updated_at = $2
                 WHERE account_id = $3 AND asset = $4",
            )
            .bind(-diff).bind(now).bind(account_id).bind(&asset)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;
        }

        // Record amendments
        if new_amount != old_amount {
            sqlx::query(
                "INSERT INTO intent_amendments (intent_id, amendment_number, field_changed, old_value, new_value, amended_by)
                 VALUES ($1, $2, 'amount_in', $3, $4, $5)",
            )
            .bind(intent_id).bind(amendment_num)
            .bind(old_amount.to_string()).bind(new_amount.to_string()).bind(&intent.user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;
        }

        let new_min_out = new_min_amount_out.map(|a| a as i64).unwrap_or(intent.min_amount_out);
        if new_min_out != intent.min_amount_out {
            sqlx::query(
                "INSERT INTO intent_amendments (intent_id, amendment_number, field_changed, old_value, new_value, amended_by)
                 VALUES ($1, $2, 'min_amount_out', $3, $4, $5)",
            )
            .bind(intent_id).bind(amendment_num)
            .bind(intent.min_amount_out.to_string()).bind(new_min_out.to_string()).bind(&intent.user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;
        }

        let final_limit_price = new_limit_price.or(intent.limit_price);
        if new_limit_price.is_some() && new_limit_price != intent.limit_price {
            sqlx::query(
                "INSERT INTO intent_amendments (intent_id, amendment_number, field_changed, old_value, new_value, amended_by)
                 VALUES ($1, $2, 'limit_price', $3, $4, $5)",
            )
            .bind(intent_id).bind(amendment_num)
            .bind(intent.limit_price.unwrap_or(0).to_string())
            .bind(new_limit_price.unwrap_or(0).to_string())
            .bind(&intent.user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;
        }

        // Update intent and reset to Open (restarts auction)
        sqlx::query(
            "UPDATE intents SET amount_in = $1, min_amount_out = $2, limit_price = $3,
                 status = 'open'
             WHERE id = $4",
        )
        .bind(new_amount).bind(new_min_out).bind(final_limit_price).bind(intent_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| IntentError::StorageError(e.to_string()))?;

        tx.commit().await.map_err(|e| IntentError::StorageError(e.to_string()))?;

        // Fetch updated intent
        let amended = self.storage.get_intent(intent_id).await.ok_or(IntentError::IntentNotFound)?;

        // Publish event
        self.event_bus
            .publish(&Event::IntentAmended(amended.clone()))
            .await?;

        tracing::info!(
            intent_id = %intent_id,
            old_amount = old_amount,
            new_amount = new_amount,
            amendment = amendment_num,
            "intent_amended"
        );

        Ok(amended)
    }
}

fn parse_asset(token: &str) -> Result<Asset, IntentError> {
    match token.to_uppercase().as_str() {
        "USDC" => Ok(Asset::USDC),
        "ETH" => Ok(Asset::ETH),
        "BTC" => Ok(Asset::BTC),
        "SOL" => Ok(Asset::SOL),
        other => Err(IntentError::InvalidAsset(other.to_string())),
    }
}
