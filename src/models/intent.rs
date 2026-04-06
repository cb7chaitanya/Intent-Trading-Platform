use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "intent_status", rename_all = "lowercase")]
pub enum IntentStatus {
    Open,
    Bidding,
    Matched,
    PartiallyFilled,
    Executing,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "order_type", rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Intent {
    pub id: Uuid,
    pub user_id: String,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: i64,
    pub min_amount_out: i64,
    pub deadline: i64,
    pub status: IntentStatus,
    pub created_at: i64,
    pub order_type: OrderType,
    pub limit_price: Option<i64>,
    pub stop_price: Option<i64>,
    pub stop_side: Option<String>,
    pub triggered_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Source chain for the input token.
    pub source_chain: String,
    /// Destination chain for the output token.
    pub destination_chain: String,
    /// True if source_chain != destination_chain.
    pub cross_chain: bool,
}

impl Intent {
    pub fn new(
        user_id: String,
        token_in: String,
        token_out: String,
        amount_in: u64,
        min_amount_out: u64,
        deadline: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            token_in,
            token_out,
            amount_in: amount_in as i64,
            min_amount_out: min_amount_out as i64,
            deadline,
            status: IntentStatus::Open,
            created_at: chrono::Utc::now().timestamp(),
            order_type: OrderType::Market,
            limit_price: None,
            stop_price: None,
            stop_side: None,
            triggered_at: None,
            source_chain: "ethereum".to_string(),
            destination_chain: "ethereum".to_string(),
            cross_chain: false,
        }
    }

    /// Set source and destination chains. Automatically sets cross_chain flag.
    pub fn with_chains(mut self, source: &str, destination: &str) -> Self {
        self.source_chain = source.to_string();
        self.destination_chain = destination.to_string();
        self.cross_chain = source != destination;
        self
    }

    pub fn with_limit(mut self, price: i64) -> Self {
        self.order_type = OrderType::Limit;
        self.limit_price = Some(price);
        self
    }

    /// Stop-loss sell: triggers when price falls to/below stop_price.
    pub fn with_stop_loss(mut self, price: i64) -> Self {
        self.order_type = OrderType::Stop;
        self.stop_price = Some(price);
        self.stop_side = Some("sell".to_string());
        self
    }

    /// Stop-buy: triggers when price rises to/above stop_price.
    pub fn with_stop_buy(mut self, price: i64) -> Self {
        self.order_type = OrderType::Stop;
        self.stop_price = Some(price);
        self.stop_side = Some("buy".to_string());
        self
    }

    /// Stop-limit: triggers at stop_price, then becomes a limit order at limit_price.
    pub fn with_stop_limit(mut self, stop: i64, limit: i64, side: &str) -> Self {
        self.order_type = OrderType::Stop;
        self.stop_price = Some(stop);
        self.limit_price = Some(limit);
        self.stop_side = Some(side.to_string());
        self
    }

    /// Legacy stop (defaults to sell/stop-loss).
    pub fn with_stop(mut self, price: i64) -> Self {
        self.order_type = OrderType::Stop;
        self.stop_price = Some(price);
        self.stop_side = Some("sell".to_string());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_intent_defaults_to_market() {
        let intent = Intent::new("u1".into(), "ETH".into(), "USDC".into(), 1000, 900, 99999);
        assert_eq!(intent.status, IntentStatus::Open);
        assert_eq!(intent.order_type, OrderType::Market);
        assert!(intent.limit_price.is_none());
        assert!(intent.stop_price.is_none());
    }

    #[test]
    fn limit_order() {
        let intent = Intent::new("u1".into(), "ETH".into(), "USDC".into(), 1000, 900, 99999)
            .with_limit(3500);
        assert_eq!(intent.order_type, OrderType::Limit);
        assert_eq!(intent.limit_price, Some(3500));
    }

    #[test]
    fn stop_order() {
        let intent = Intent::new("u1".into(), "ETH".into(), "USDC".into(), 1000, 900, 99999)
            .with_stop(3000);
        assert_eq!(intent.order_type, OrderType::Stop);
        assert_eq!(intent.stop_price, Some(3000));
    }
}
