use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a single leg of a cross-chain settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "leg_status", rename_all = "lowercase")]
pub enum LegStatus {
    /// Leg created, awaiting execution.
    Pending,
    /// Funds locked in escrow on the source chain.
    Escrowed,
    /// Transaction submitted to chain.
    Executing,
    /// Transaction confirmed on chain.
    Confirmed,
    /// Transaction failed on chain.
    Failed,
    /// Timeout expired; funds returned to user.
    Refunded,
}

/// One leg of a cross-chain settlement (source or destination).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrossChainLeg {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub fill_id: Uuid,
    /// 0 = source chain (lock/escrow), 1 = destination chain (release).
    pub leg_index: i16,
    pub chain: String,
    pub from_address: String,
    pub to_address: String,
    pub token_mint: Option<String>,
    pub amount: i64,
    pub tx_hash: Option<String>,
    pub status: LegStatus,
    pub error: Option<String>,
    /// If both legs are not confirmed by this time, trigger refund.
    pub timeout_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

/// Summary view of a cross-chain settlement.
#[derive(Debug, Clone, Serialize)]
pub struct CrossChainSettlement {
    pub intent_id: Uuid,
    pub fill_id: Uuid,
    pub source_leg: CrossChainLeg,
    pub destination_leg: CrossChainLeg,
    pub fully_confirmed: bool,
    pub timed_out: bool,
}

impl CrossChainSettlement {
    pub fn from_legs(legs: Vec<CrossChainLeg>) -> Option<Self> {
        let source = legs.iter().find(|l| l.leg_index == 0)?.clone();
        let dest = legs.iter().find(|l| l.leg_index == 1)?.clone();

        let fully_confirmed =
            source.status == LegStatus::Confirmed && dest.status == LegStatus::Confirmed;

        let now = Utc::now();
        let timed_out = !fully_confirmed
            && (source.timeout_at < now || dest.timeout_at < now);

        Some(Self {
            intent_id: source.intent_id,
            fill_id: source.fill_id,
            source_leg: source,
            destination_leg: dest,
            fully_confirmed,
            timed_out,
        })
    }
}
