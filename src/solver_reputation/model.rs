use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Solver {
    pub id: Uuid,
    pub name: String,
    pub successful_trades: i64,
    pub failed_trades: i64,
    pub total_volume: i64,
    pub reputation_score: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct TopSolversQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    10
}
