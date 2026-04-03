use std::sync::Arc;

use uuid::Uuid;

use crate::db::redis::{Event, EventBus};
use crate::db::storage::Storage;
use crate::models::intent::{Intent, IntentStatus};

pub struct IntentService {
    storage: Arc<Storage>,
    event_bus: EventBus,
}

impl IntentService {
    pub fn new(storage: Arc<Storage>, event_bus: EventBus) -> Self {
        Self {
            storage,
            event_bus,
        }
    }

    pub async fn create_intent(
        &mut self,
        user_id: String,
        token_in: String,
        token_out: String,
        amount_in: u64,
        min_amount_out: u64,
        deadline: i64,
    ) -> Result<Intent, redis::RedisError> {
        let intent = Intent::new(user_id, token_in, token_out, amount_in, min_amount_out, deadline);
        self.storage.insert_intent(intent.clone());
        self.event_bus
            .publish(&Event::IntentCreated(intent.clone()))
            .await?;
        Ok(intent)
    }

    pub fn get_intent(&self, intent_id: &Uuid) -> Option<Intent> {
        self.storage.get_intent(intent_id)
    }

    pub fn list_intents(&self) -> Vec<Intent> {
        self.storage.list_intents()
    }

    pub fn update_intent_status(
        &self,
        intent_id: &Uuid,
        status: IntentStatus,
    ) -> Option<Intent> {
        let mut intent = self.storage.get_intent(intent_id)?;
        intent.status = status;
        self.storage.update_intent(intent.clone());
        Some(intent)
    }

    pub async fn cancel_intent(
        &mut self,
        intent_id: &Uuid,
    ) -> Result<Option<Intent>, redis::RedisError> {
        let Some(mut intent) = self.storage.get_intent(intent_id) else {
            return Ok(None);
        };
        intent.status = IntentStatus::Cancelled;
        self.storage.update_intent(intent.clone());
        self.event_bus
            .publish(&Event::IntentCancelled(intent.clone()))
            .await?;
        Ok(Some(intent))
    }

    pub async fn start_bidding(
        &mut self,
        intent_id: &Uuid,
    ) -> Result<Option<Intent>, redis::RedisError> {
        let Some(mut intent) = self.storage.get_intent(intent_id) else {
            return Ok(None);
        };
        intent.status = IntentStatus::Bidding;
        self.storage.update_intent(intent.clone());
        self.event_bus
            .publish(&Event::IntentBidding(intent.clone()))
            .await?;
        Ok(Some(intent))
    }
}
