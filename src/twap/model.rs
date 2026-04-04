use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "twap_status", rename_all = "lowercase")]
pub enum TwapStatus {
    Active,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TwapIntent {
    pub id: Uuid,
    pub user_id: String,
    pub account_id: Uuid,
    pub token_in: String,
    pub token_out: String,
    pub total_qty: i64,
    pub filled_qty: i64,
    pub min_price: i64,
    pub duration_secs: i64,
    pub interval_secs: i64,
    pub slices_total: i32,
    pub slices_completed: i32,
    pub status: TwapStatus,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TwapChildIntent {
    pub id: Uuid,
    pub twap_id: Uuid,
    pub intent_id: Uuid,
    pub slice_index: i32,
    pub qty: i64,
    pub status: String,
    pub scheduled_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTwapRequest {
    pub user_id: String,
    pub account_id: Uuid,
    pub token_in: String,
    pub token_out: String,
    pub total_qty: i64,
    pub min_price: i64,
    pub duration_secs: i64,
    pub interval_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct TwapProgress {
    pub twap_id: Uuid,
    pub status: TwapStatus,
    pub total_qty: i64,
    pub filled_qty: i64,
    pub slices_total: i32,
    pub slices_completed: i32,
    pub remaining_qty: i64,
    pub pct_complete: f64,
}
