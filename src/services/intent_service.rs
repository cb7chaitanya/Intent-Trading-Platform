use std::sync::Arc;

use uuid::Uuid;

use crate::metrics::counters;
use crate::balances::model::Asset;
use crate::balances::service::{BalanceError, BalanceService};
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
    balance_service: Arc<BalanceService>,
    risk_engine: Arc<RiskEngine>,
}

impl IntentService {
    pub fn new(
        storage: Arc<Storage>,
        event_bus: EventBus,
        stream_bus: Arc<StreamBus>,
        balance_service: Arc<BalanceService>,
        risk_engine: Arc<RiskEngine>,
    ) -> Self {
        Self {
            storage,
            event_bus,
            stream_bus,
            balance_service,
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
        self.balance_service
            .lock_balance(account_id, asset, amount_in as i64)
            .await
            .map_err(|e| match e {
                BalanceError::InsufficientBalance => IntentError::InsufficientBalance,
                other => IntentError::BalanceError(other.to_string()),
            })?;

        self.risk_engine
            .record_accepted_intent(&user_id, amount_in)
            .await;

        let intent = Intent::new(
            user_id, token_in, token_out, amount_in, min_amount_out, deadline,
        );
        self.storage
            .insert_intent(&intent)
            .await
            .map_err(|e| IntentError::StorageError(e.to_string()))?;

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
        self.balance_service
            .unlock_balance(account_id, asset, intent.amount_in)
            .await
            .map_err(|e| IntentError::BalanceError(e.to_string()))?;

        intent.status = IntentStatus::Cancelled;
        let _ = self.storage.update_intent(&intent).await;
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
        self.balance_service
            .unlock_balance(account_id, asset, intent.amount_in)
            .await
            .map_err(|e| IntentError::BalanceError(e.to_string()))?;

        intent.status = IntentStatus::Failed;
        let _ = self.storage.update_intent(&intent).await;
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
