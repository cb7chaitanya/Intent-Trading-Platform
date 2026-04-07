//! HTLC swap lifecycle management.
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────┐
//!  │                   HTLC Atomic Swap Flow                     │
//!  │                                                             │
//!  │  1. create_swap()                                           │
//!  │     Platform generates secret S, computes H = SHA256(S)     │
//!  │     Stores H in DB, keeps S in memory until source lock     │
//!  │                                                             │
//!  │  2. record_source_lock(swap_id, tx_hash)                    │
//!  │     User/platform locks funds on source chain:              │
//!  │     "Pay X tokens to solver IF they reveal preimage of H    │
//!  │      within T seconds; otherwise refund to sender"          │
//!  │                                                             │
//!  │  3. Solver sees the source lock and creates a mirror HTLC   │
//!  │     on the destination chain with the SAME hash H but a     │
//!  │     SHORTER timelock (T/2). This is off-chain / solver's    │
//!  │     responsibility — we track it via record_dest_lock().     │
//!  │                                                             │
//!  │  4. record_dest_claim(swap_id, secret, claim_tx)            │
//!  │     User/platform claims the dest HTLC by revealing S.      │
//!  │     Now the secret is public on the dest chain.             │
//!  │                                                             │
//!  │  5. complete_swap(swap_id, unlock_tx)                        │
//!  │     Solver (or anyone) uses the now-public S to unlock      │
//!  │     the source HTLC and receive the source chain funds.     │
//!  │                                                             │
//!  │  Timeout path:                                              │
//!  │     If dest claim doesn't happen before T/2, solver's dest  │
//!  │     lock expires and solver gets dest funds back.           │
//!  │     If source unlock doesn't happen before T, user gets     │
//!  │     source funds back via refund_swap().                    │
//!  └─────────────────────────────────────────────────────────────┘
//! ```

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::crypto;
use super::model::{CreateHtlcParams, HtlcStatus, HtlcSwap};

/// Default source timelock: 30 minutes.
const DEFAULT_TIMELOCK_SECS: i64 = 1800;

#[derive(Debug)]
pub enum HtlcError {
    NotFound,
    InvalidState(String),
    SecretMismatch,
    Expired,
    DbError(sqlx::Error),
}

impl std::fmt::Display for HtlcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtlcError::NotFound => write!(f, "HTLC swap not found"),
            HtlcError::InvalidState(s) => write!(f, "Invalid HTLC state: {s}"),
            HtlcError::SecretMismatch => write!(f, "Secret does not match hash"),
            HtlcError::Expired => write!(f, "HTLC timelock expired"),
            HtlcError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for HtlcError {
    fn from(e: sqlx::Error) -> Self {
        HtlcError::DbError(e)
    }
}

pub struct HtlcService {
    pool: PgPool,
}

impl HtlcService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Step 1: Create swap ──────────────────────────

    /// Generate secret + hash, create the HTLC record.
    /// Returns (swap, secret) — caller must use the secret to build
    /// the source chain lock transaction.
    pub async fn create_swap(
        &self,
        params: CreateHtlcParams,
    ) -> Result<(HtlcSwap, crypto::Secret), HtlcError> {
        let secret = crypto::generate_secret();
        let secret_hash = crypto::hash_secret(&secret);

        let timelock_secs = if params.timelock_secs > 0 {
            params.timelock_secs
        } else {
            DEFAULT_TIMELOCK_SECS
        };

        let now = Utc::now();
        let timelock = now + Duration::seconds(timelock_secs);

        let swap = HtlcSwap {
            id: Uuid::new_v4(),
            fill_id: params.fill_id,
            intent_id: params.intent_id,
            secret_hash: secret_hash.to_vec(),
            secret: None, // not stored until dest claim
            source_chain: params.source_chain,
            source_sender: params.source_sender,
            source_receiver: params.source_receiver,
            source_token: params.source_token,
            source_amount: params.source_amount,
            source_lock_tx: None,
            source_unlock_tx: None,
            source_timelock: timelock,
            dest_chain: params.dest_chain,
            dest_sender: params.dest_sender,
            dest_receiver: params.dest_receiver,
            dest_token: params.dest_token,
            dest_amount: params.dest_amount,
            dest_lock_tx: None,
            dest_claim_tx: None,
            status: HtlcStatus::Created,
            solver_id: params.solver_id,
            error: None,
            created_at: now,
            locked_at: None,
            claimed_at: None,
            completed_at: None,
        };

        sqlx::query(
            "INSERT INTO htlc_swaps
                (id, fill_id, intent_id, secret_hash, source_chain, source_sender,
                 source_receiver, source_token, source_amount, source_timelock,
                 dest_chain, dest_sender, dest_receiver, dest_token, dest_amount,
                 status, solver_id, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
        )
        .bind(swap.id)
        .bind(swap.fill_id)
        .bind(swap.intent_id)
        .bind(&swap.secret_hash)
        .bind(&swap.source_chain)
        .bind(&swap.source_sender)
        .bind(&swap.source_receiver)
        .bind(&swap.source_token)
        .bind(swap.source_amount)
        .bind(swap.source_timelock)
        .bind(&swap.dest_chain)
        .bind(&swap.dest_sender)
        .bind(&swap.dest_receiver)
        .bind(&swap.dest_token)
        .bind(swap.dest_amount)
        .bind(&swap.status)
        .bind(&swap.solver_id)
        .bind(swap.created_at)
        .execute(&self.pool)
        .await?;

        tracing::info!(
            htlc_id = %swap.id,
            fill_id = %swap.fill_id,
            source_chain = %swap.source_chain,
            dest_chain = %swap.dest_chain,
            source_amount = swap.source_amount,
            timelock = %swap.source_timelock,
            hash = %crypto::to_hex(&secret_hash),
            "htlc_created"
        );

        Ok((swap, secret))
    }

    // ── Step 1b: Store secret for later retrieval ─────

    /// Persist the secret in the DB so the worker can retrieve it
    /// when claiming the destination HTLC.
    ///
    /// The secret is stored as raw bytes. In production you would
    /// encrypt it with a KMS key before writing, but the column
    /// already exists (`secret BYTEA`) and this avoids the need
    /// for a separate secret vault while keeping the worker
    /// self-contained.
    ///
    /// Must be called right after `create_swap` returns the secret
    /// to the caller — before any status transition.
    pub async fn store_secret(
        &self,
        swap_id: Uuid,
        secret: &crypto::Secret,
    ) -> Result<(), HtlcError> {
        // Verify the secret matches the stored hash before persisting
        let swap = self.get_swap(swap_id).await?;
        let expected: crypto::SecretHash = swap.secret_hash
            .try_into()
            .map_err(|_| HtlcError::InvalidState("Stored hash has wrong length".into()))?;

        if !crypto::verify_secret(secret, &expected) {
            return Err(HtlcError::SecretMismatch);
        }

        sqlx::query(
            "UPDATE htlc_swaps SET secret = $2 WHERE id = $1 AND status = 'created'",
        )
        .bind(swap_id)
        .bind(secret.as_slice())
        .execute(&self.pool)
        .await?;

        tracing::info!(
            htlc_id = %swap_id,
            "htlc_secret_stored"
        );

        Ok(())
    }

    // ── Step 2: Record source lock ───────────────────

    /// Source chain HTLC has been deployed and confirmed.
    pub async fn record_source_lock(
        &self,
        swap_id: Uuid,
        lock_tx: &str,
    ) -> Result<(), HtlcError> {
        let result = sqlx::query(
            "UPDATE htlc_swaps
             SET status = 'source_locked', source_lock_tx = $2, locked_at = NOW()
             WHERE id = $1 AND status = 'created'",
        )
        .bind(swap_id)
        .bind(lock_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState("Expected status: created".into()));
        }

        tracing::info!(htlc_id = %swap_id, lock_tx, "htlc_source_locked");
        Ok(())
    }

    // ── Step 3: Record solver's dest lock ────────────

    /// Solver has created their mirror HTLC on the destination chain.
    pub async fn record_dest_lock(
        &self,
        swap_id: Uuid,
        dest_lock_tx: &str,
    ) -> Result<(), HtlcError> {
        let result = sqlx::query(
            "UPDATE htlc_swaps SET dest_lock_tx = $2
             WHERE id = $1 AND status = 'source_locked'",
        )
        .bind(swap_id)
        .bind(dest_lock_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState(
                "Expected status: source_locked".into(),
            ));
        }

        tracing::info!(htlc_id = %swap_id, dest_lock_tx, "htlc_dest_locked");
        Ok(())
    }

    // ── Step 4: Claim destination (reveal secret) ────

    /// User/platform claims the destination HTLC by revealing the secret.
    /// This makes the secret public on-chain.
    pub async fn record_dest_claim(
        &self,
        swap_id: Uuid,
        secret: &crypto::Secret,
        claim_tx: &str,
    ) -> Result<(), HtlcError> {
        let swap = self.get_swap(swap_id).await?;

        // Verify the secret matches the stored hash
        let expected: crypto::SecretHash = swap.secret_hash
            .try_into()
            .map_err(|_| HtlcError::InvalidState("Stored hash has wrong length".into()))?;

        if !crypto::verify_secret(secret, &expected) {
            return Err(HtlcError::SecretMismatch);
        }

        let result = sqlx::query(
            "UPDATE htlc_swaps
             SET status = 'dest_claimed', secret = $2, dest_claim_tx = $3, claimed_at = NOW()
             WHERE id = $1 AND status = 'source_locked'",
        )
        .bind(swap_id)
        .bind(secret.as_slice())
        .bind(claim_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState(
                "Expected status: source_locked".into(),
            ));
        }

        tracing::info!(
            htlc_id = %swap_id,
            claim_tx,
            secret_hex = %crypto::to_hex(secret),
            "htlc_dest_claimed_secret_revealed"
        );

        Ok(())
    }

    /// Record destination claim using the stored secret.
    ///
    /// The worker calls this when it has the secret from the DB
    /// (stored via `store_secret`) and has submitted the claim tx
    /// to the destination chain. The secret is verified against the
    /// hash before updating.
    pub async fn record_dest_claim_with_stored_secret(
        &self,
        swap_id: Uuid,
        claim_tx: &str,
    ) -> Result<(), HtlcError> {
        let swap = self.get_swap(swap_id).await?;

        let secret_bytes = swap.secret.ok_or_else(|| {
            HtlcError::InvalidState("Secret not stored — call store_secret first".into())
        })?;

        let secret: crypto::Secret = secret_bytes
            .try_into()
            .map_err(|_| HtlcError::InvalidState("Stored secret has wrong length".into()))?;

        let expected: crypto::SecretHash = swap.secret_hash
            .try_into()
            .map_err(|_| HtlcError::InvalidState("Stored hash has wrong length".into()))?;

        if !crypto::verify_secret(&secret, &expected) {
            return Err(HtlcError::SecretMismatch);
        }

        let result = sqlx::query(
            "UPDATE htlc_swaps
             SET status = 'dest_claimed', dest_claim_tx = $2, claimed_at = NOW()
             WHERE id = $1 AND status = 'source_locked'",
        )
        .bind(swap_id)
        .bind(claim_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState(
                "Expected status: source_locked".into(),
            ));
        }

        tracing::info!(
            htlc_id = %swap_id,
            claim_tx,
            secret_hex = %crypto::to_hex(&secret),
            "htlc_dest_claimed_with_verified_secret"
        );

        Ok(())
    }

    /// Extract secret from on-chain claim event logs.
    ///
    /// The Solana HTLC program emits a `FundsClaimed` event with the
    /// secret (preimage) when someone claims the destination HTLC.
    /// For EVM, the claim tx log contains the secret in the event data.
    ///
    /// This parses the secret from a hex-encoded log data field.
    pub fn extract_secret_from_logs(
        log_data: &str,
    ) -> Result<crypto::Secret, HtlcError> {
        // Strip "0x" prefix if present, then decode hex
        let hex_str = log_data.strip_prefix("0x").unwrap_or(log_data);

        // The secret is 32 bytes. In Solana Anchor events it's at
        // bytes [40..72] of the event data (after 8-byte discriminator
        // + 32-byte htlc pubkey). In EVM it's the first 32 bytes of
        // the event data field. Handle both:
        let data = hex::decode(hex_str).map_err(|e| {
            HtlcError::InvalidState(format!("Invalid hex in claim log: {e}"))
        })?;

        if data.len() < 32 {
            return Err(HtlcError::InvalidState(format!(
                "Claim log data too short: {} bytes, need >= 32",
                data.len()
            )));
        }

        // Try to find a 32-byte secret that matches.
        // For EVM: first 32 bytes are the secret.
        // For Solana: bytes 40..72 after discriminator + pubkey.
        // We return the first 32 bytes — the caller should pass the
        // correct slice for their chain.
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&data[..32]);
        Ok(secret)
    }

    /// Record destination claim without verifying the secret.
    /// Used by the worker when the claim happens via bridge relay
    /// rather than direct preimage submission.
    #[deprecated(note = "Use record_dest_claim or record_dest_claim_with_stored_secret instead")]
    pub async fn record_dest_claim_unchecked(
        &self,
        swap_id: Uuid,
        claim_tx: &str,
    ) -> Result<(), HtlcError> {
        let result = sqlx::query(
            "UPDATE htlc_swaps
             SET status = 'dest_claimed', dest_claim_tx = $2, claimed_at = NOW()
             WHERE id = $1 AND status = 'source_locked'",
        )
        .bind(swap_id)
        .bind(claim_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState(
                "Expected status: source_locked".into(),
            ));
        }

        tracing::info!(htlc_id = %swap_id, claim_tx, "htlc_dest_claimed_unchecked");
        Ok(())
    }

    // ── Step 5: Complete (unlock source) ─────────────

    /// Solver used the revealed secret to unlock the source HTLC.
    /// The atomic swap is complete.
    pub async fn complete_swap(
        &self,
        swap_id: Uuid,
        unlock_tx: &str,
    ) -> Result<(), HtlcError> {
        let result = sqlx::query(
            "UPDATE htlc_swaps
             SET status = 'source_unlocked', source_unlock_tx = $2, completed_at = NOW()
             WHERE id = $1 AND status = 'dest_claimed'",
        )
        .bind(swap_id)
        .bind(unlock_tx)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState(
                "Expected status: dest_claimed".into(),
            ));
        }

        tracing::info!(htlc_id = %swap_id, unlock_tx, "htlc_completed");
        Ok(())
    }

    // ── Timeout / refund ─────────────────────────────

    /// Refund an expired HTLC. The source timelock must have passed.
    pub async fn refund_swap(&self, swap_id: Uuid) -> Result<(), HtlcError> {
        let swap = self.get_swap(swap_id).await?;

        if Utc::now() < swap.source_timelock {
            return Err(HtlcError::InvalidState(
                "Timelock has not expired yet".into(),
            ));
        }

        if swap.status == HtlcStatus::DestClaimed || swap.status == HtlcStatus::SourceUnlocked {
            return Err(HtlcError::InvalidState(
                "Cannot refund: secret already revealed".into(),
            ));
        }

        let result = sqlx::query(
            "UPDATE htlc_swaps SET status = 'refunded', completed_at = NOW()
             WHERE id = $1 AND status IN ('created', 'source_locked')",
        )
        .bind(swap_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(HtlcError::InvalidState("Not in refundable state".into()));
        }

        tracing::info!(htlc_id = %swap_id, "htlc_refunded");
        Ok(())
    }

    /// Mark as failed with error message.
    pub async fn fail_swap(&self, swap_id: Uuid, error: &str) -> Result<(), HtlcError> {
        sqlx::query(
            "UPDATE htlc_swaps SET status = 'failed', error = $2, completed_at = NOW()
             WHERE id = $1 AND status NOT IN ('source_unlocked', 'refunded')",
        )
        .bind(swap_id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Queries ──────────────────────────────────────

    pub async fn get_swap(&self, swap_id: Uuid) -> Result<HtlcSwap, HtlcError> {
        sqlx::query_as::<_, HtlcSwap>("SELECT * FROM htlc_swaps WHERE id = $1")
            .bind(swap_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(HtlcError::NotFound)
    }

    pub async fn get_swap_by_fill(&self, fill_id: Uuid) -> Result<Option<HtlcSwap>, HtlcError> {
        Ok(
            sqlx::query_as::<_, HtlcSwap>("SELECT * FROM htlc_swaps WHERE fill_id = $1")
                .bind(fill_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Find swaps that need source locking (Created, not timed out).
    pub async fn find_pending_locks(&self) -> Result<Vec<HtlcSwap>, HtlcError> {
        let now = Utc::now();
        Ok(sqlx::query_as::<_, HtlcSwap>(
            "SELECT * FROM htlc_swaps
             WHERE status = 'created' AND source_timelock > $1
             ORDER BY created_at ASC LIMIT 50",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Find swaps where source is locked and solver should have created dest lock.
    pub async fn find_awaiting_dest_lock(&self) -> Result<Vec<HtlcSwap>, HtlcError> {
        Ok(sqlx::query_as::<_, HtlcSwap>(
            "SELECT * FROM htlc_swaps
             WHERE status = 'source_locked' AND dest_lock_tx IS NULL
             ORDER BY locked_at ASC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Find swaps where dest is locked and ready for claiming.
    pub async fn find_claimable(&self) -> Result<Vec<HtlcSwap>, HtlcError> {
        Ok(sqlx::query_as::<_, HtlcSwap>(
            "SELECT * FROM htlc_swaps
             WHERE status = 'source_locked' AND dest_lock_tx IS NOT NULL
             ORDER BY locked_at ASC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Find swaps where dest was claimed (secret revealed) but source not yet unlocked.
    pub async fn find_pending_unlocks(&self) -> Result<Vec<HtlcSwap>, HtlcError> {
        Ok(sqlx::query_as::<_, HtlcSwap>(
            "SELECT * FROM htlc_swaps
             WHERE status = 'dest_claimed' AND secret IS NOT NULL
             ORDER BY claimed_at ASC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Find swaps past their timelock that should be refunded.
    pub async fn find_expired(&self) -> Result<Vec<HtlcSwap>, HtlcError> {
        let now = Utc::now();
        Ok(sqlx::query_as::<_, HtlcSwap>(
            "SELECT * FROM htlc_swaps
             WHERE source_timelock < $1
               AND status IN ('created', 'source_locked')
             ORDER BY source_timelock ASC LIMIT 50",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }
}
