//! Chaos test reporting — tracks injected faults and system responses.
//!
//! Logs a summary at shutdown showing which faults fired, how many times,
//! and whether the system recovered (based on circuit breaker + worker state).

use std::collections::HashMap;

use super::faults::FaultKind;

/// Accumulates chaos test statistics.
pub struct ChaosReport {
    activations: HashMap<String, u64>,
    started_at: std::time::Instant,
}

impl ChaosReport {
    pub fn new() -> Self {
        Self {
            activations: HashMap::new(),
            started_at: std::time::Instant::now(),
        }
    }

    pub fn record_activation(&mut self, kind: &FaultKind) {
        *self.activations.entry(kind.as_str().to_string()).or_default() += 1;
    }

    pub fn total_injections(&self) -> u64 {
        self.activations.values().sum()
    }

    pub fn log_summary(&self) {
        let elapsed = self.started_at.elapsed();

        tracing::info!(
            "╔══════════════════════════════════════════════════╗"
        );
        tracing::info!(
            "║           CHAOS TEST REPORT                      ║"
        );
        tracing::info!(
            "╠══════════════════════════════════════════════════╣"
        );
        tracing::info!(
            duration_secs = elapsed.as_secs(),
            total_injections = self.total_injections(),
            "chaos_report_summary"
        );

        for (fault, count) in &self.activations {
            tracing::info!(
                fault = %fault,
                activations = count,
                "chaos_report_fault"
            );
        }

        tracing::info!(
            "╠══════════════════════════════════════════════════╣"
        );
        tracing::info!(
            "║  Check circuit_breaker_state metrics for recovery ║"
        );
        tracing::info!(
            "║  Check htlc_swaps_total for swap integrity        ║"
        );
        tracing::info!(
            "║  Check cross_chain_legs for settlement status      ║"
        );
        tracing::info!(
            "╚══════════════════════════════════════════════════╝"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_accumulates() {
        let mut report = ChaosReport::new();
        report.record_activation(&FaultKind::EthRpcTimeout);
        report.record_activation(&FaultKind::EthRpcTimeout);
        report.record_activation(&FaultKind::BridgeFailure);
        assert_eq!(report.total_injections(), 3);
        assert_eq!(report.activations["eth_rpc_timeout"], 2);
        assert_eq!(report.activations["bridge_failure"], 1);
    }

    #[test]
    fn empty_report() {
        let report = ChaosReport::new();
        assert_eq!(report.total_injections(), 0);
    }
}
