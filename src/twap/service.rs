use std::sync::Arc;

use chrono::{Duration, Utc};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::services::intent_service::IntentService;

use super::model::*;

#[derive(Debug)]
pub enum TwapError {
    InvalidParams(String),
    NotFound,
    AlreadyCancelled,
    DbError(sqlx::Error),
    IntentError(String),
}

impl std::fmt::Display for TwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TwapError::InvalidParams(e) => write!(f, "Invalid params: {e}"),
            TwapError::NotFound => write!(f, "TWAP intent not found"),
            TwapError::AlreadyCancelled => write!(f, "TWAP already cancelled"),
            TwapError::DbError(e) => write!(f, "Database error: {e}"),
            TwapError::IntentError(e) => write!(f, "Intent error: {e}"),
        }
    }
}

impl From<sqlx::Error> for TwapError {
    fn from(e: sqlx::Error) -> Self {
        TwapError::DbError(e)
    }
}

pub struct TwapService {
    pool: PgPool,
    intent_service: Arc<Mutex<IntentService>>,
}

impl TwapService {
    pub fn new(pool: PgPool, intent_service: Arc<Mutex<IntentService>>) -> Self {
        Self { pool, intent_service }
    }

    /// Create a TWAP intent and generate child intent schedule.
    pub async fn create_twap(&self, req: CreateTwapRequest) -> Result<TwapIntent, TwapError> {
        if req.interval_secs <= 0 || req.duration_secs <= 0 {
            return Err(TwapError::InvalidParams("duration and interval must be positive".into()));
        }
        if req.total_qty <= 0 {
            return Err(TwapError::InvalidParams("total_qty must be positive".into()));
        }
        if req.interval_secs > req.duration_secs {
            return Err(TwapError::InvalidParams("interval cannot exceed duration".into()));
        }

        let slices_total = (req.duration_secs / req.interval_secs) as i32;
        let qty_per_slice = req.total_qty / slices_total as i64;
        let remainder = req.total_qty - (qty_per_slice * slices_total as i64);

        let now = Utc::now();
        let id = Uuid::new_v4();

        let twap = TwapIntent {
            id,
            user_id: req.user_id.clone(),
            account_id: req.account_id,
            token_in: req.token_in.clone(),
            token_out: req.token_out.clone(),
            total_qty: req.total_qty,
            filled_qty: 0,
            min_price: req.min_price,
            duration_secs: req.duration_secs,
            interval_secs: req.interval_secs,
            slices_total,
            slices_completed: 0,
            status: TwapStatus::Active,
            created_at: now,
            finished_at: None,
        };

        // Insert parent
        sqlx::query(
            "INSERT INTO twap_intents (id, user_id, account_id, token_in, token_out, total_qty,
                filled_qty, min_price, duration_secs, interval_secs, slices_total, slices_completed,
                status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(twap.id).bind(&twap.user_id).bind(twap.account_id)
        .bind(&twap.token_in).bind(&twap.token_out).bind(twap.total_qty)
        .bind(twap.filled_qty).bind(twap.min_price)
        .bind(twap.duration_secs).bind(twap.interval_secs)
        .bind(twap.slices_total).bind(twap.slices_completed)
        .bind(&twap.status).bind(twap.created_at)
        .execute(&self.pool).await?;

        // Generate child intent schedule
        for i in 0..slices_total {
            let scheduled_at = now + Duration::seconds(req.interval_secs * i as i64);
            let slice_qty = if i == slices_total - 1 { qty_per_slice + remainder } else { qty_per_slice };
            let intent_id = Uuid::new_v4(); // placeholder until actually submitted

            sqlx::query(
                "INSERT INTO twap_child_intents (twap_id, intent_id, slice_index, qty, status, scheduled_at)
                 VALUES ($1, $2, $3, $4, 'pending', $5)",
            )
            .bind(id).bind(intent_id).bind(i).bind(slice_qty).bind(scheduled_at)
            .execute(&self.pool).await?;
        }

        tracing::info!(
            twap_id = %id,
            user_id = %req.user_id,
            total_qty = req.total_qty,
            slices = slices_total,
            interval_secs = req.interval_secs,
            "twap_created"
        );

        Ok(twap)
    }

    /// Cancel a TWAP and all its pending child intents.
    pub async fn cancel_twap(&self, twap_id: Uuid, account_id: Uuid) -> Result<TwapIntent, TwapError> {
        let twap = self.get_twap(twap_id).await?.ok_or(TwapError::NotFound)?;

        if twap.status != TwapStatus::Active {
            return Err(TwapError::AlreadyCancelled);
        }

        // Cancel all pending children
        let pending = sqlx::query_as::<_, TwapChildIntent>(
            "SELECT * FROM twap_child_intents WHERE twap_id = $1 AND status = 'pending'",
        )
        .bind(twap_id).fetch_all(&self.pool).await?;

        for child in &pending {
            // Cancel the actual intent if it was submitted
            let mut svc = self.intent_service.lock().await;
            let _ = svc.cancel_intent(&child.intent_id, account_id).await;
        }

        // Mark children as cancelled
        sqlx::query(
            "UPDATE twap_child_intents SET status = 'cancelled' WHERE twap_id = $1 AND status = 'pending'",
        )
        .bind(twap_id).execute(&self.pool).await?;

        // Mark parent as cancelled
        let now = Utc::now();
        sqlx::query(
            "UPDATE twap_intents SET status = 'cancelled', finished_at = $1 WHERE id = $2",
        )
        .bind(now).bind(twap_id).execute(&self.pool).await?;

        tracing::info!(twap_id = %twap_id, "twap_cancelled");

        Ok(TwapIntent { status: TwapStatus::Cancelled, finished_at: Some(now), ..twap })
    }

    /// Get TWAP progress.
    pub async fn get_progress(&self, twap_id: Uuid) -> Result<TwapProgress, TwapError> {
        let twap = self.get_twap(twap_id).await?.ok_or(TwapError::NotFound)?;

        let pct = if twap.total_qty > 0 {
            (twap.filled_qty as f64 / twap.total_qty as f64) * 100.0
        } else { 0.0 };

        Ok(TwapProgress {
            twap_id: twap.id,
            status: twap.status,
            total_qty: twap.total_qty,
            filled_qty: twap.filled_qty,
            slices_total: twap.slices_total,
            slices_completed: twap.slices_completed,
            remaining_qty: twap.total_qty - twap.filled_qty,
            pct_complete: (pct * 100.0).round() / 100.0,
        })
    }

    pub async fn get_twap(&self, id: Uuid) -> Result<Option<TwapIntent>, TwapError> {
        Ok(sqlx::query_as::<_, TwapIntent>("SELECT * FROM twap_intents WHERE id = $1")
            .bind(id).fetch_optional(&self.pool).await?)
    }

    /// Record a child intent completion. Updates parent progress.
    pub async fn record_child_completed(
        &self,
        twap_id: Uuid,
        child_id: Uuid,
        filled_qty: i64,
    ) -> Result<(), TwapError> {
        sqlx::query("UPDATE twap_child_intents SET status = 'completed' WHERE id = $1")
            .bind(child_id).execute(&self.pool).await?;

        sqlx::query(
            "UPDATE twap_intents SET filled_qty = filled_qty + $1, slices_completed = slices_completed + 1 WHERE id = $2",
        )
        .bind(filled_qty).bind(twap_id).execute(&self.pool).await?;

        // Check if all slices done
        let twap = self.get_twap(twap_id).await?.ok_or(TwapError::NotFound)?;
        if twap.slices_completed >= twap.slices_total {
            sqlx::query("UPDATE twap_intents SET status = 'completed', finished_at = NOW() WHERE id = $1")
                .bind(twap_id).execute(&self.pool).await?;
            tracing::info!(twap_id = %twap_id, filled_qty = twap.filled_qty, "twap_completed");
        }

        Ok(())
    }

    pub async fn record_child_failed(
        &self,
        child_id: Uuid,
    ) -> Result<(), TwapError> {
        sqlx::query("UPDATE twap_child_intents SET status = 'failed' WHERE id = $1")
            .bind(child_id).execute(&self.pool).await?;
        Ok(())
    }
}
