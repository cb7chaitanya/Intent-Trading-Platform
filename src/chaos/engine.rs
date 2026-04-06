//! Chaos engine — schedules fault injection based on probability and duration.
//!
//! Activated by CHAOS_ENABLED=true environment variable.
//! Runs as a background task alongside the main application.
//! Randomly triggers faults, holds them for a configured duration,
//! then deactivates and logs recovery behavior.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio_util::sync::CancellationToken;

use super::faults::{FaultConfig, FaultKind, FaultRegistry};
use super::report::ChaosReport;

/// Default chaos schedule: all fault types with conservative probabilities.
pub fn default_schedule() -> Vec<FaultConfig> {
    vec![
        FaultConfig {
            kind: FaultKind::EthRpcTimeout,
            probability: 0.05,
            duration: Duration::from_secs(10),
            target: None,
        },
        FaultConfig {
            kind: FaultKind::SolanaRpcFailure,
            probability: 0.05,
            duration: Duration::from_secs(8),
            target: None,
        },
        FaultConfig {
            kind: FaultKind::RedisDisconnect,
            probability: 0.03,
            duration: Duration::from_secs(5),
            target: None,
        },
        FaultConfig {
            kind: FaultKind::PostgresFailover,
            probability: 0.02,
            duration: Duration::from_secs(15),
            target: None,
        },
        FaultConfig {
            kind: FaultKind::BridgeFailure,
            probability: 0.05,
            duration: Duration::from_secs(20),
            target: None,
        },
        FaultConfig {
            kind: FaultKind::TransactionDropped,
            probability: 0.08,
            duration: Duration::from_secs(0), // instant, per-tx
            target: None,
        },
        FaultConfig {
            kind: FaultKind::ChainReorg,
            probability: 0.02,
            duration: Duration::from_secs(0), // instant event
            target: None,
        },
        FaultConfig {
            kind: FaultKind::WorkerCrash,
            probability: 0.03,
            duration: Duration::from_secs(10),
            target: None,
        },
    ]
}

const TICK_INTERVAL_SECS: u64 = 5;

/// Check if chaos mode should be enabled based on environment.
pub fn is_chaos_enabled() -> bool {
    std::env::var("CHAOS_ENABLED")
        .unwrap_or_default()
        .eq_ignore_ascii_case("true")
}

/// Run the chaos engine. Periodically evaluates each fault's probability,
/// activates faults, and deactivates after their duration expires.
pub async fn run(
    registry: Arc<FaultRegistry>,
    schedule: Vec<FaultConfig>,
    cancel: CancellationToken,
) {
    if !is_chaos_enabled() {
        tracing::info!("Chaos testing disabled (set CHAOS_ENABLED=true to enable)");
        return;
    }

    registry.enable();
    tracing::warn!(
        faults = schedule.len(),
        tick_secs = TICK_INTERVAL_SECS,
        "CHAOS ENGINE ACTIVE — fault injection enabled"
    );

    let mut report = ChaosReport::new();

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(TICK_INTERVAL_SECS)) => {
                for config in &schedule {
                    let fault = registry.get_fault(&config.kind);

                    if fault.is_active() {
                        continue;
                    }

                    let roll: f64 = rand::random();
                    if roll < config.probability {
                        // Trigger the fault
                        fault.activate();
                        report.record_activation(&config.kind);

                        tracing::warn!(
                            fault = config.kind.as_str(),
                            probability = config.probability,
                            duration_secs = config.duration.as_secs(),
                            roll,
                            "chaos_fault_injected"
                        );

                        // Schedule deactivation after duration
                        if config.duration > Duration::ZERO {
                            let fault_clone = Arc::clone(fault);
                            let kind_name = config.kind.as_str().to_string();
                            let dur = config.duration;
                            tokio::spawn(async move {
                                tokio::time::sleep(dur).await;
                                fault_clone.deactivate();
                                tracing::info!(
                                    fault = %kind_name,
                                    "chaos_fault_deactivated"
                                );
                            });
                        } else {
                            // Instant fault — deactivate immediately
                            // (the activation flag was seen by concurrent checks)
                            fault.deactivate();
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                // Deactivate all faults on shutdown
                for config in &schedule {
                    registry.get_fault(&config.kind).deactivate();
                }
                tracing::info!(
                    total_injections = report.total_injections(),
                    "Chaos engine shutting down"
                );
                report.log_summary();
                return;
            }
        }
    }
}
