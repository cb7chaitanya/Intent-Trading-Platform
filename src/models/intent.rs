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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_intent_defaults_to_open() {
        let intent = Intent::new(
            "user1".into(), "ETH".into(), "USDC".into(), 1000, 900, 9999999999,
        );
        assert_eq!(intent.status, IntentStatus::Open);
        assert_eq!(intent.amount_in, 1000);
        assert_eq!(intent.min_amount_out, 900);
        assert!(!intent.id.is_nil());
        assert!(intent.created_at > 0);
    }

    #[test]
    fn intent_serializes_to_json() {
        let intent = Intent::new(
            "u1".into(), "BTC".into(), "USDC".into(), 500, 400, 123,
        );
        let json = serde_json::to_string(&intent).unwrap();
        assert!(json.contains("\"token_in\":\"BTC\""));
        assert!(json.contains("\"status\":\"Open\""));
    }
}
