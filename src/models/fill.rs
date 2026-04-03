use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub intent_id: Uuid,
    pub solver_id: String,
    pub price: u64,
    pub qty: u64,
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
            price,
            qty,
            tx_hash,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
