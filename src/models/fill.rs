use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Fill {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,
    pub price: i64,
    pub qty: i64,
    pub filled_qty: i64,
    pub tx_hash: String,
    pub timestamp: i64,
    pub settled: bool,
    pub settled_at: Option<DateTime<Utc>>,
}

impl Fill {
    pub fn new(
        intent_id: Uuid,
        solver_id: String,
        price: i64,
        qty: i64,
        filled_qty: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            solver_id,
            price,
            qty,
            filled_qty,
            tx_hash: String::new(),
            timestamp: chrono::Utc::now().timestamp(),
            settled: false,
            settled_at: None,
        }
    }
}
