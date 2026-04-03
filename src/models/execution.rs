use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionStatus {
    Pending,
    Executing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,
    pub tx_hash: String,
    pub status: ExecutionStatus,
    pub created_at: i64,
}

impl Execution {
    pub fn new(
        intent_id: Uuid,
        solver_id: String,
        tx_hash: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            solver_id,
            tx_hash,
            status: ExecutionStatus::Pending,
            created_at: chrono::Utc::now().timestamp(),
        }
    }
}
