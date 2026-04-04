use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Fill {
    pub intent_id: Uuid,
    pub solver_id: String,
    pub price: i64,
    pub qty: i64,
    pub tx_hash: String,
    pub timestamp: i64,
}

impl Fill {
    pub fn new(
        intent_id: Uuid,
        solver_id: String,
        price: u64,
        qty: u64,
        tx_hash: String,
    ) -> Self {
        Self {
            intent_id,
            solver_id,
            price: price as i64,
            qty: qty as i64,
            tx_hash,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
