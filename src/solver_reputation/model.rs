use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Solver {
    pub id: Uuid,
    pub name: String,
    pub email: Option<String>,
    pub api_key: Option<String>,
    pub webhook_url: Option<String>,
    pub active: bool,
    pub successful_trades: i64,
    pub failed_trades: i64,
    pub total_volume: i64,
    pub total_fills: i64,
    pub failed_fills: i64,
    pub reputation_score: f64,
    pub created_at: DateTime<Utc>,
}

/// Public view that hides the api_key.
#[derive(Debug, Clone, Serialize)]
pub struct SolverPublic {
    pub id: Uuid,
    pub name: String,
    pub active: bool,
    pub successful_trades: i64,
    pub failed_trades: i64,
    pub total_volume: i64,
    pub total_fills: i64,
    pub failed_fills: i64,
    pub reputation_score: f64,
    pub created_at: DateTime<Utc>,
}

impl From<Solver> for SolverPublic {
    fn from(s: Solver) -> Self {
        Self {
            id: s.id,
            name: s.name,
            active: s.active,
            successful_trades: s.successful_trades,
            failed_trades: s.failed_trades,
            total_volume: s.total_volume,
            total_fills: s.total_fills,
            failed_fills: s.failed_fills,
            reputation_score: s.reputation_score,
            created_at: s.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RegisterSolverRequest {
    pub name: String,
    pub email: Option<String>,
    pub webhook_url: Option<String>,
}

/// Returned once on registration so the solver can store its key.
#[derive(Debug, Serialize)]
pub struct RegisterSolverResponse {
    pub solver_id: Uuid,
    pub api_key: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSolverRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SolverDashboard {
    pub solver: SolverPublic,
    pub win_rate: f64,
    pub fill_success_rate: f64,
    pub avg_volume_per_trade: f64,
}

#[derive(Debug, Deserialize)]
pub struct TopSolversQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    10
}
