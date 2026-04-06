use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// HTLC swap lifecycle.
///
/// ```text
///  Created ──► SourceLocked ──► DestClaimed ──► SourceUnlocked
///     │              │                               (done)
///     │              └──► (timeout) ──► Refunded
///     └──► Failed
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "htlc_status", rename_all = "snake_case")]
pub enum HtlcStatus {
    /// Secret generated, awaiting source chain lock.
    Created,
    /// Funds locked on source chain with hash timelock.
    SourceLocked,
    /// Solver claimed on destination chain, revealing the secret.
    DestClaimed,
    /// Platform used the revealed secret to unlock source funds. Terminal success.
    SourceUnlocked,
    /// Timelock expired, funds returned to sender. Terminal.
    Refunded,
    /// Timelock expired without any action. Terminal.
    Expired,
    /// Unrecoverable error. Terminal.
    Failed,
}

impl HtlcStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            HtlcStatus::SourceUnlocked
                | HtlcStatus::Refunded
                | HtlcStatus::Expired
                | HtlcStatus::Failed
        )
    }
}

/// A single HTLC atomic swap record.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HtlcSwap {
    pub id: Uuid,
    pub fill_id: Uuid,
    pub intent_id: Uuid,

    /// SHA-256(secret). Published on both chains as the lock condition.
    pub secret_hash: Vec<u8>,
    /// The preimage. Only stored after the solver reveals it.
    pub secret: Option<Vec<u8>>,

    // Source chain (user → solver)
    pub source_chain: String,
    pub source_sender: String,
    pub source_receiver: String,
    pub source_token: Option<String>,
    pub source_amount: i64,
    pub source_lock_tx: Option<String>,
    pub source_unlock_tx: Option<String>,
    pub source_timelock: DateTime<Utc>,

    // Destination chain (solver → user)
    pub dest_chain: String,
    pub dest_sender: String,
    pub dest_receiver: String,
    pub dest_token: Option<String>,
    pub dest_amount: i64,
    pub dest_lock_tx: Option<String>,
    pub dest_claim_tx: Option<String>,

    pub status: HtlcStatus,
    pub solver_id: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub locked_at: Option<DateTime<Utc>>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Parameters to initiate an HTLC swap.
#[derive(Debug, Clone)]
pub struct CreateHtlcParams {
    pub fill_id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,

    pub source_chain: String,
    pub source_sender: String,
    pub source_receiver: String,
    pub source_token: Option<String>,
    pub source_amount: i64,

    pub dest_chain: String,
    pub dest_sender: String,
    pub dest_receiver: String,
    pub dest_token: Option<String>,
    pub dest_amount: i64,

    /// How long the source HTLC stays locked (seconds).
    pub timelock_secs: i64,
}
