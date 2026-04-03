use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::market_data::model::{MarketTrade, OrderBookSnapshot};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum WsFeedEvent {
    Trade(MarketTrade),
    OrderBook(OrderBookSnapshot),
    AuctionResult {
        intent_id: Uuid,
        winner_solver_id: String,
        amount_out: u64,
        fee: u64,
    },
    Subscribed {
        market_id: Uuid,
    },
    Unsubscribed {
        market_id: Uuid,
    },
    Pong,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum ClientMessage {
    #[serde(rename = "subscribe")]
    Subscribe { market_id: Uuid },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { market_id: Uuid },
    #[serde(rename = "ping")]
    Ping,
}

/// Per-market broadcast channel.
struct MarketChannel {
    tx: broadcast::Sender<String>,
}

impl MarketChannel {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self { tx }
    }
}

/// Tracks which markets each client is subscribed to.
#[derive(Clone)]
pub struct WsFeed {
    /// market_id → broadcast channel
    channels: Arc<RwLock<HashMap<Uuid, MarketChannel>>>,
    /// Global broadcast for non-market events (Redis relay, auction results)
    global_tx: broadcast::Sender<String>,
}

impl WsFeed {
    pub fn new() -> Self {
        let (global_tx, _) = broadcast::channel(512);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            global_tx,
        }
    }

    /// Get or create a broadcast channel for a market.
    async fn get_or_create_channel(
        &self,
        market_id: &Uuid,
    ) -> broadcast::Sender<String> {
        {
            let channels = self.channels.read().await;
            if let Some(ch) = channels.get(market_id) {
                return ch.tx.clone();
            }
        }
        let mut channels = self.channels.write().await;
        let ch = channels
            .entry(*market_id)
            .or_insert_with(MarketChannel::new);
        ch.tx.clone()
    }

    /// Subscribe to a market's broadcast channel.
    pub async fn subscribe_market(
        &self,
        market_id: &Uuid,
    ) -> broadcast::Receiver<String> {
        let tx = self.get_or_create_channel(market_id).await;
        tx.subscribe()
    }

    /// Broadcast a trade to all clients watching that market.
    pub async fn broadcast_trade(&self, trade: &MarketTrade) {
        let event = WsFeedEvent::Trade(trade.clone());
        if let Ok(json) = serde_json::to_string(&event) {
            let tx = self.get_or_create_channel(&trade.market_id).await;
            let _ = tx.send(json);
        }
    }

    /// Broadcast an orderbook snapshot to all clients watching that market.
    pub async fn broadcast_orderbook(&self, snapshot: &OrderBookSnapshot) {
        let event = WsFeedEvent::OrderBook(snapshot.clone());
        if let Ok(json) = serde_json::to_string(&event) {
            let tx = self.get_or_create_channel(&snapshot.market_id).await;
            let _ = tx.send(json);
        }
    }

    /// Broadcast an auction result globally.
    pub fn broadcast_global(&self, message: &str) {
        let _ = self.global_tx.send(message.to_string());
    }

    pub fn subscribe_global(&self) -> broadcast::Receiver<String> {
        self.global_tx.subscribe()
    }
}

/// Tracks subscriptions for a single WebSocket client session.
pub struct ClientSession {
    pub id: Uuid,
    pub subscriptions: HashSet<Uuid>,
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            subscriptions: HashSet::new(),
        }
    }
}
