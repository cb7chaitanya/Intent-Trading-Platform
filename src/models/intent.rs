use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "intent_status", rename_all = "lowercase")]
pub enum IntentStatus {
    Open,
    Bidding,
    Matched,
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
