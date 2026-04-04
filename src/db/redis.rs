use std::future::Future;

use futures_util::StreamExt;
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};

use crate::models::bid::SolverBid;
use crate::models::execution::Execution;
use crate::models::intent::Intent;

pub const INTENT_CREATED: &str = "intent_created";
pub const BID_SUBMITTED: &str = "bid_submitted";
pub const INTENT_MATCHED: &str = "intent_matched";
pub const EXECUTION_STARTED: &str = "execution_started";
pub const EXECUTION_COMPLETED: &str = "execution_completed";
pub const EXECUTION_FAILED: &str = "execution_failed";
pub const INTENT_CANCELLED: &str = "intent_cancelled";
pub const INTENT_BIDDING: &str = "intent_bidding";
pub const INTENT_FAILED: &str = "intent_failed";
pub const INTENT_EXPIRED: &str = "intent_expired";

pub const ALL_CHANNELS: &[&str] = &[
    INTENT_CREATED,
    BID_SUBMITTED,
    INTENT_MATCHED,
    EXECUTION_STARTED,
    EXECUTION_COMPLETED,
    EXECUTION_FAILED,
    INTENT_CANCELLED,
    INTENT_BIDDING,
    INTENT_FAILED,
    INTENT_EXPIRED,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    IntentCreated(Intent),
    BidSubmitted(SolverBid),
    IntentMatched { intent: Intent, bid: SolverBid },
    ExecutionStarted(Execution),
    ExecutionCompleted(Execution),
    ExecutionFailed { execution: Execution, reason: String },
    IntentCancelled(Intent),
    IntentBidding(Intent),
    IntentFailed(Intent),
    IntentExpired(Intent),
}

impl Event {
    pub fn channel(&self) -> &'static str {
        match self {
            Event::IntentCreated(_) => INTENT_CREATED,
            Event::BidSubmitted(_) => BID_SUBMITTED,
            Event::IntentMatched { .. } => INTENT_MATCHED,
            Event::ExecutionStarted(_) => EXECUTION_STARTED,
            Event::ExecutionCompleted(_) => EXECUTION_COMPLETED,
            Event::ExecutionFailed { .. } => EXECUTION_FAILED,
            Event::IntentCancelled(_) => INTENT_CANCELLED,
            Event::IntentBidding(_) => INTENT_BIDDING,
            Event::IntentFailed(_) => INTENT_FAILED,
            Event::IntentExpired(_) => INTENT_EXPIRED,
        }
    }
}

pub struct EventBus {
    client: Client,
    publisher: MultiplexedConnection,
}

impl EventBus {
    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = Client::open(redis_url)?;
        let publisher = client.get_multiplexed_async_connection().await?;
        Ok(Self { client, publisher })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn publish(&mut self, event: &Event) -> Result<u32, redis::RedisError> {
        let channel = event.channel();
        let message = serde_json::to_string(event).map_err(|e| {
            redis::RedisError::from((
                redis::ErrorKind::TypeError,
                "JSON serialization failed",
                e.to_string(),
            ))
        })?;
        let receivers: u32 = self.publisher.publish(channel, &message).await?;
        Ok(receivers)
    }

    pub async fn subscribe<F, Fut>(
        &self,
        channels: &[&str],
        handler: F,
    ) -> Result<(), redis::RedisError>
    where
        F: Fn(String, Event) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut pubsub = self.client.get_async_pubsub().await?;

        for channel in channels {
            pubsub.subscribe(*channel).await?;
        }

        let mut stream = pubsub.on_message();
        while let Some(msg) = stream.next().await {
            let channel: String = msg.get_channel_name().to_string();
            let payload: String = msg.get_payload()?;

            match serde_json::from_str::<Event>(&payload) {
                Ok(event) => handler(channel, event).await,
                Err(e) => tracing::warn!(channel = %channel, error = %e, "Failed to deserialize event"),
            }
        }

        Ok(())
    }
}
