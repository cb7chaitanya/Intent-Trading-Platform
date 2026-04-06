//! Fault injection definitions for chaos testing.
//!
//! Each fault type describes a specific failure mode. The chaos engine
//! activates faults randomly based on probability and duration.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Fault types ──────────────────────────────────────────

/// All supported failure injection scenarios.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// Ethereum RPC calls timeout (no response within deadline).
    EthRpcTimeout,
    /// Solana RPC returns errors for all calls.
    SolanaRpcFailure,
    /// Redis connection drops (pub/sub, cache, rate limiter).
    RedisDisconnect,
    /// Postgres becomes unreachable (simulates failover).
    PostgresFailover,
    /// Bridge adapter calls fail (Wormhole/LayerZero).
    BridgeFailure,
    /// Submitted transactions disappear from mempool.
    TransactionDropped,
    /// Chain reorganization: confirmed blocks become unconfirmed.
    ChainReorg,
    /// Background worker panics and must be restarted.
    WorkerCrash,
}

impl FaultKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FaultKind::EthRpcTimeout => "eth_rpc_timeout",
            FaultKind::SolanaRpcFailure => "solana_rpc_failure",
            FaultKind::RedisDisconnect => "redis_disconnect",
            FaultKind::PostgresFailover => "postgres_failover",
            FaultKind::BridgeFailure => "bridge_failure",
            FaultKind::TransactionDropped => "transaction_dropped",
            FaultKind::ChainReorg => "chain_reorg",
            FaultKind::WorkerCrash => "worker_crash",
        }
    }
}

// ── Fault configuration ──────────────────────────────────

/// Configuration for a single fault injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultConfig {
    pub kind: FaultKind,
    /// Probability of the fault firing on each check (0.0 to 1.0).
    pub probability: f64,
    /// How long the fault stays active once triggered.
    pub duration: Duration,
    /// Optional: only affect a specific service/chain.
    pub target: Option<String>,
}

// ── Active fault state ───────────────────────────────────

/// Runtime state for an active fault.
pub struct ActiveFault {
    pub kind: FaultKind,
    pub active: AtomicBool,
    pub triggered_count: AtomicU64,
    pub target: Option<String>,
}

impl ActiveFault {
    pub fn new(kind: FaultKind, target: Option<String>) -> Self {
        Self {
            kind,
            active: AtomicBool::new(false),
            triggered_count: AtomicU64::new(0),
            target,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn activate(&self) {
        self.active.store(true, Ordering::Relaxed);
        self.triggered_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn deactivate(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    pub fn trigger_count(&self) -> u64 {
        self.triggered_count.load(Ordering::Relaxed)
    }
}

// ── Global fault registry ────────────────────────────────

/// Thread-safe registry of all injectable faults. Checked by system
/// components at key decision points.
pub struct FaultRegistry {
    pub eth_rpc_timeout: Arc<ActiveFault>,
    pub solana_rpc_failure: Arc<ActiveFault>,
    pub redis_disconnect: Arc<ActiveFault>,
    pub postgres_failover: Arc<ActiveFault>,
    pub bridge_failure: Arc<ActiveFault>,
    pub transaction_dropped: Arc<ActiveFault>,
    pub chain_reorg: Arc<ActiveFault>,
    pub worker_crash: Arc<ActiveFault>,
    enabled: AtomicBool,
}

impl FaultRegistry {
    pub fn new() -> Self {
        Self {
            eth_rpc_timeout: Arc::new(ActiveFault::new(FaultKind::EthRpcTimeout, None)),
            solana_rpc_failure: Arc::new(ActiveFault::new(FaultKind::SolanaRpcFailure, None)),
            redis_disconnect: Arc::new(ActiveFault::new(FaultKind::RedisDisconnect, None)),
            postgres_failover: Arc::new(ActiveFault::new(FaultKind::PostgresFailover, None)),
            bridge_failure: Arc::new(ActiveFault::new(FaultKind::BridgeFailure, None)),
            transaction_dropped: Arc::new(ActiveFault::new(FaultKind::TransactionDropped, None)),
            chain_reorg: Arc::new(ActiveFault::new(FaultKind::ChainReorg, None)),
            worker_crash: Arc::new(ActiveFault::new(FaultKind::WorkerCrash, None)),
            enabled: AtomicBool::new(false),
        }
    }

    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Check if a specific fault should fire right now.
    /// Returns false if chaos mode is disabled globally.
    pub fn should_fault(&self, kind: &FaultKind) -> bool {
        if !self.is_enabled() {
            return false;
        }
        match kind {
            FaultKind::EthRpcTimeout => self.eth_rpc_timeout.is_active(),
            FaultKind::SolanaRpcFailure => self.solana_rpc_failure.is_active(),
            FaultKind::RedisDisconnect => self.redis_disconnect.is_active(),
            FaultKind::PostgresFailover => self.postgres_failover.is_active(),
            FaultKind::BridgeFailure => self.bridge_failure.is_active(),
            FaultKind::TransactionDropped => self.transaction_dropped.is_active(),
            FaultKind::ChainReorg => self.chain_reorg.is_active(),
            FaultKind::WorkerCrash => self.worker_crash.is_active(),
        }
    }

    pub fn get_fault(&self, kind: &FaultKind) -> &Arc<ActiveFault> {
        match kind {
            FaultKind::EthRpcTimeout => &self.eth_rpc_timeout,
            FaultKind::SolanaRpcFailure => &self.solana_rpc_failure,
            FaultKind::RedisDisconnect => &self.redis_disconnect,
            FaultKind::PostgresFailover => &self.postgres_failover,
            FaultKind::BridgeFailure => &self.bridge_failure,
            FaultKind::TransactionDropped => &self.transaction_dropped,
            FaultKind::ChainReorg => &self.chain_reorg,
            FaultKind::WorkerCrash => &self.worker_crash,
        }
    }

    /// Get summary of all fault states.
    pub fn snapshot(&self) -> Vec<(&'static str, bool, u64)> {
        let kinds = [
            FaultKind::EthRpcTimeout,
            FaultKind::SolanaRpcFailure,
            FaultKind::RedisDisconnect,
            FaultKind::PostgresFailover,
            FaultKind::BridgeFailure,
            FaultKind::TransactionDropped,
            FaultKind::ChainReorg,
            FaultKind::WorkerCrash,
        ];
        kinds
            .iter()
            .map(|k| {
                let f = self.get_fault(k);
                (k.as_str(), f.is_active(), f.trigger_count())
            })
            .collect()
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_disabled() {
        let reg = FaultRegistry::new();
        assert!(!reg.is_enabled());
        assert!(!reg.should_fault(&FaultKind::EthRpcTimeout));
    }

    #[test]
    fn enabled_but_no_active_faults() {
        let reg = FaultRegistry::new();
        reg.enable();
        assert!(!reg.should_fault(&FaultKind::EthRpcTimeout));
    }

    #[test]
    fn activate_fault() {
        let reg = FaultRegistry::new();
        reg.enable();
        reg.eth_rpc_timeout.activate();
        assert!(reg.should_fault(&FaultKind::EthRpcTimeout));
        assert!(!reg.should_fault(&FaultKind::SolanaRpcFailure));
    }

    #[test]
    fn deactivate_fault() {
        let reg = FaultRegistry::new();
        reg.enable();
        reg.bridge_failure.activate();
        assert!(reg.should_fault(&FaultKind::BridgeFailure));
        reg.bridge_failure.deactivate();
        assert!(!reg.should_fault(&FaultKind::BridgeFailure));
    }

    #[test]
    fn trigger_count_increments() {
        let fault = ActiveFault::new(FaultKind::WorkerCrash, None);
        assert_eq!(fault.trigger_count(), 0);
        fault.activate();
        assert_eq!(fault.trigger_count(), 1);
        fault.deactivate();
        fault.activate();
        assert_eq!(fault.trigger_count(), 2);
    }

    #[test]
    fn snapshot_lists_all_faults() {
        let reg = FaultRegistry::new();
        reg.enable();
        reg.chain_reorg.activate();
        let snap = reg.snapshot();
        assert_eq!(snap.len(), 8);
        let reorg = snap.iter().find(|s| s.0 == "chain_reorg").unwrap();
        assert!(reorg.1); // active
        assert_eq!(reorg.2, 1); // triggered once
    }

    #[test]
    fn fault_kind_as_str() {
        assert_eq!(FaultKind::EthRpcTimeout.as_str(), "eth_rpc_timeout");
        assert_eq!(FaultKind::ChainReorg.as_str(), "chain_reorg");
    }
}
