use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SolverBid {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,
    pub amount_out: i64,
    pub fee: i64,
    pub timestamp: i64,
}

impl SolverBid {
    pub fn new(
        intent_id: Uuid,
        solver_id: String,
        amount_out: u64,
        fee: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            solver_id,
            amount_out: amount_out as i64,
            fee: fee as i64,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
