use std::sync::Arc;

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::wallet::chain::ChainAdapter;
use crate::wallet::registry::ChainRegistry;

use super::model::{CrossChainLeg, CrossChainSettlement, LegStatus};

/// Default timeout for cross-chain settlement (10 minutes).
const DEFAULT_TIMEOUT_SECS: i64 = 600;

/// Maximum solver collateral multiplier for cross-chain intents.
pub const CROSS_CHAIN_COLLATERAL_MULTIPLIER: f64 = 1.5;

#[derive(Debug)]
pub enum CrossChainError {
    LegNotFound,
    InvalidState(String),
    ChainError(String),
    Timeout,
    DbError(sqlx::Error),
}

impl std::fmt::Display for CrossChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrossChainError::LegNotFound => write!(f, "Cross-chain leg not found"),
            CrossChainError::InvalidState(s) => write!(f, "Invalid state: {s}"),
            CrossChainError::ChainError(e) => write!(f, "Chain error: {e}"),
            CrossChainError::Timeout => write!(f, "Cross-chain settlement timed out"),
            CrossChainError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for CrossChainError {
    fn from(e: sqlx::Error) -> Self {
        CrossChainError::DbError(e)
    }
}

pub struct CrossChainService {
    pool: PgPool,
    chains: Arc<ChainRegistry>,
}

impl CrossChainService {
    pub fn new(pool: PgPool, chains: Arc<ChainRegistry>) -> Self {
        Self { pool, chains }
    }

    // ── Create legs ──────────────────────────────────

    /// Create both legs for a cross-chain settlement.
    /// Source leg: lock funds on source_chain.
    /// Destination leg: release funds on destination_chain.
    pub async fn create_settlement(
        &self,
        intent_id: Uuid,
        fill_id: Uuid,
        source_chain: &str,
        dest_chain: &str,
        source_from: &str,
        source_to: &str,
        dest_from: &str,
        dest_to: &str,
        source_mint: Option<&str>,
        dest_mint: Option<&str>,
        amount: i64,
    ) -> Result<(CrossChainLeg, CrossChainLeg), CrossChainError> {
        let now = Utc::now();
        let timeout = now + Duration::seconds(DEFAULT_TIMEOUT_SECS);

        let source_leg = CrossChainLeg {
            id: Uuid::new_v4(),
            intent_id,
            fill_id,
            leg_index: 0,
            chain: source_chain.to_string(),
            from_address: source_from.to_string(),
            to_address: source_to.to_string(),
            token_mint: source_mint.map(String::from),
            amount,
            tx_hash: None,
            status: LegStatus::Pending,
            error: None,
            timeout_at: timeout,
            created_at: now,
            confirmed_at: None,
        };

        let dest_leg = CrossChainLeg {
            id: Uuid::new_v4(),
            intent_id,
            fill_id,
            leg_index: 1,
            chain: dest_chain.to_string(),
            from_address: dest_from.to_string(),
            to_address: dest_to.to_string(),
            token_mint: dest_mint.map(String::from),
            amount,
            tx_hash: None,
            status: LegStatus::Pending,
            error: None,
            timeout_at: timeout,
            created_at: now,
            confirmed_at: None,
        };

        self.insert_leg(&source_leg).await?;
        self.insert_leg(&dest_leg).await?;

        tracing::info!(
            intent_id = %intent_id,
            fill_id = %fill_id,
            source_chain,
            dest_chain,
            timeout = %timeout,
            "cross_chain_settlement_created"
        );

        Ok((source_leg, dest_leg))
    }

    // ── Execute legs ─────────────────────────────────

    /// Execute the source leg: lock funds in escrow on the source chain.
    pub async fn execute_source_leg(
        &self,
        leg_id: Uuid,
        tx_hash: &str,
    ) -> Result<(), CrossChainError> {
        self.update_leg_status(leg_id, LegStatus::Escrowed, Some(tx_hash), None)
            .await
    }

    /// Mark a leg as executing (transaction submitted).
    pub async fn mark_executing(
        &self,
        leg_id: Uuid,
        tx_hash: &str,
    ) -> Result<(), CrossChainError> {
        self.update_leg_status(leg_id, LegStatus::Executing, Some(tx_hash), None)
            .await
    }

    /// Mark a leg as confirmed (transaction finalized on chain).
    pub async fn confirm_leg(&self, leg_id: Uuid) -> Result<(), CrossChainError> {
        self.update_leg_status(leg_id, LegStatus::Confirmed, None, None)
            .await?;

        // Update confirmed_at
        sqlx::query("UPDATE cross_chain_legs SET confirmed_at = NOW() WHERE id = $1")
            .bind(leg_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Mark a leg as failed.
    pub async fn fail_leg(&self, leg_id: Uuid, error: &str) -> Result<(), CrossChainError> {
        self.update_leg_status(leg_id, LegStatus::Failed, None, Some(error))
            .await
    }

    /// Refund a leg (timeout expired, funds returned).
    pub async fn refund_leg(&self, leg_id: Uuid) -> Result<(), CrossChainError> {
        self.update_leg_status(leg_id, LegStatus::Refunded, None, Some("Timeout refund"))
            .await
    }

    // ── Query ────────────────────────────────────────

    /// Get both legs for a fill and compute settlement status.
    pub async fn get_settlement(
        &self,
        fill_id: Uuid,
    ) -> Result<Option<CrossChainSettlement>, CrossChainError> {
        let legs = sqlx::query_as::<_, CrossChainLeg>(
            "SELECT * FROM cross_chain_legs WHERE fill_id = $1 ORDER BY leg_index",
        )
        .bind(fill_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(CrossChainSettlement::from_legs(legs))
    }

    /// Find source legs (leg_index=0) still in Pending state — need bridge.lock_funds.
    pub async fn find_pending_source_legs(&self) -> Result<Vec<CrossChainLeg>, CrossChainError> {
        let legs = sqlx::query_as::<_, CrossChainLeg>(
            "SELECT * FROM cross_chain_legs
             WHERE leg_index = 0 AND status = 'pending'
             ORDER BY created_at ASC
             LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(legs)
    }

    /// Find source legs (leg_index=0) in Escrowed state — need bridge.verify_lock.
    pub async fn find_escrowed_source_legs(&self) -> Result<Vec<CrossChainLeg>, CrossChainError> {
        let legs = sqlx::query_as::<_, CrossChainLeg>(
            "SELECT * FROM cross_chain_legs
             WHERE leg_index = 0 AND status = 'escrowed'
             ORDER BY created_at ASC
             LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(legs)
    }

    /// Find all legs that have timed out and need refunding.
    pub async fn find_timed_out_legs(&self) -> Result<Vec<CrossChainLeg>, CrossChainError> {
        let now = Utc::now();
        let legs = sqlx::query_as::<_, CrossChainLeg>(
            "SELECT * FROM cross_chain_legs
             WHERE timeout_at < $1
               AND status NOT IN ('confirmed', 'refunded')
             ORDER BY timeout_at ASC
             LIMIT 50",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(legs)
    }

    /// Find legs that are escrowed and whose counterpart on the other chain
    /// has been confirmed — these are ready for destination execution.
    pub async fn find_ready_destination_legs(&self) -> Result<Vec<CrossChainLeg>, CrossChainError> {
        let legs = sqlx::query_as::<_, CrossChainLeg>(
            "SELECT dest.*
             FROM cross_chain_legs dest
             JOIN cross_chain_legs src ON src.fill_id = dest.fill_id AND src.leg_index = 0
             WHERE dest.leg_index = 1
               AND dest.status = 'pending'
               AND src.status IN ('escrowed', 'confirmed')
             ORDER BY dest.created_at ASC
             LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(legs)
    }

    // ── Solver risk constraints ──────────────────────

    /// Check if a solver has enough margin for a cross-chain fill.
    /// Cross-chain fills require CROSS_CHAIN_COLLATERAL_MULTIPLIER * amount
    /// because the solver bears the bridge risk.
    pub fn required_collateral(&self, amount: i64) -> i64 {
        (amount as f64 * CROSS_CHAIN_COLLATERAL_MULTIPLIER) as i64
    }

    /// Validate that a solver can handle a cross-chain execution.
    pub async fn validate_solver_for_cross_chain(
        &self,
        solver_id: &str,
        source_chain: &str,
        dest_chain: &str,
    ) -> Result<(), CrossChainError> {
        // Verify both chains are supported
        self.chains.get(source_chain).map_err(|e| {
            CrossChainError::ChainError(format!("Source chain unsupported: {e}"))
        })?;
        self.chains.get(dest_chain).map_err(|e| {
            CrossChainError::ChainError(format!("Destination chain unsupported: {e}"))
        })?;

        // Check solver doesn't have too many pending cross-chain settlements
        let pending_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM cross_chain_legs l
             JOIN fills f ON f.id = l.fill_id
             WHERE f.solver_id = $1
               AND l.status IN ('pending', 'escrowed', 'executing')
               AND l.leg_index = 0",
        )
        .bind(solver_id)
        .fetch_one(&self.pool)
        .await?;

        if pending_count >= 10 {
            return Err(CrossChainError::InvalidState(
                "Solver has too many pending cross-chain settlements".into(),
            ));
        }

        Ok(())
    }

    // ── Internal helpers ─────────────────────────────

    async fn insert_leg(&self, leg: &CrossChainLeg) -> Result<(), CrossChainError> {
        sqlx::query(
            "INSERT INTO cross_chain_legs
                (id, intent_id, fill_id, leg_index, chain, from_address, to_address,
                 token_mint, amount, tx_hash, status, error, timeout_at, created_at, confirmed_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
        )
        .bind(leg.id)
        .bind(leg.intent_id)
        .bind(leg.fill_id)
        .bind(leg.leg_index)
        .bind(&leg.chain)
        .bind(&leg.from_address)
        .bind(&leg.to_address)
        .bind(&leg.token_mint)
        .bind(leg.amount)
        .bind(&leg.tx_hash)
        .bind(&leg.status)
        .bind(&leg.error)
        .bind(leg.timeout_at)
        .bind(leg.created_at)
        .bind(leg.confirmed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_leg_status(
        &self,
        leg_id: Uuid,
        status: LegStatus,
        tx_hash: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), CrossChainError> {
        let result = sqlx::query(
            "UPDATE cross_chain_legs
             SET status = $2,
                 tx_hash = COALESCE($3, tx_hash),
                 error = COALESCE($4, error)
             WHERE id = $1",
        )
        .bind(leg_id)
        .bind(&status)
        .bind(tx_hash)
        .bind(error)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CrossChainError::LegNotFound);
        }

        tracing::info!(
            leg_id = %leg_id,
            status = ?status,
            "cross_chain_leg_updated"
        );

        Ok(())
    }
}
