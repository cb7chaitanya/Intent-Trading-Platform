use sqlx::PgPool;
use uuid::Uuid;

use crate::models::bid::SolverBid;
use crate::models::execution::Execution;
use crate::models::fill::Fill;
use crate::models::intent::{Intent, IntentStatus};

pub struct Storage {
    pool: PgPool,
}

impl Storage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // --- Intents ---

    pub async fn insert_intent(&self, intent: &Intent) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO intents (id, user_id, token_in, token_out, amount_in, min_amount_out, deadline, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(intent.id)
        .bind(&intent.user_id)
        .bind(&intent.token_in)
        .bind(&intent.token_out)
        .bind(intent.amount_in)
        .bind(intent.min_amount_out)
        .bind(intent.deadline)
        .bind(&intent.status)
        .bind(intent.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_intent(&self, id: &Uuid) -> Option<Intent> {
        sqlx::query_as::<_, Intent>("SELECT * FROM intents WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
    }

    pub async fn list_intents(&self) -> Vec<Intent> {
        sqlx::query_as::<_, Intent>("SELECT * FROM intents ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
    }

    pub async fn list_active_intents(&self) -> Vec<Intent> {
        sqlx::query_as::<_, Intent>(
            "SELECT * FROM intents WHERE status IN ('open', 'bidding', 'matched', 'executing') ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    pub async fn update_intent(&self, intent: &Intent) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
            .bind(&intent.status)
            .bind(intent.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- Bids ---

    pub async fn insert_bid(&self, bid: &SolverBid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO bids (id, intent_id, solver_id, amount_out, fee, timestamp)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(bid.id)
        .bind(bid.intent_id)
        .bind(&bid.solver_id)
        .bind(bid.amount_out)
        .bind(bid.fee)
        .bind(bid.timestamp)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_bids(&self, intent_id: &Uuid) -> Vec<SolverBid> {
        sqlx::query_as::<_, SolverBid>(
            "SELECT * FROM bids WHERE intent_id = $1 ORDER BY timestamp ASC",
        )
        .bind(intent_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    // --- Fills ---

    pub async fn insert_fill(&self, fill: &Fill) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO fills (id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled, settled_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(fill.id)
        .bind(fill.intent_id)
        .bind(&fill.solver_id)
        .bind(fill.price)
        .bind(fill.qty)
        .bind(fill.filled_qty)
        .bind(&fill.tx_hash)
        .bind(fill.timestamp)
        .bind(fill.settled)
        .bind(fill.settled_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_fills(&self, intent_id: &Uuid) -> Vec<Fill> {
        sqlx::query_as::<_, Fill>(
            "SELECT * FROM fills WHERE intent_id = $1 ORDER BY price DESC",
        )
        .bind(intent_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    // --- Executions ---

    pub async fn insert_execution(&self, execution: &Execution) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO executions (id, intent_id, solver_id, tx_hash, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(execution.id)
        .bind(execution.intent_id)
        .bind(&execution.solver_id)
        .bind(&execution.tx_hash)
        .bind(&execution.status)
        .bind(execution.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_execution(&self, execution: &Execution) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE executions SET status = $1 WHERE id = $2")
            .bind(&execution.status)
            .bind(execution.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
