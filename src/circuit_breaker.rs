//! Circuit breaker for external service calls (RPC nodes, price APIs).
//!
//! States:
//!   Closed  — requests pass through, failures counted
//!   Open    — requests rejected immediately, no external calls
//!   HalfOpen — one probe request allowed to test recovery
//!
//! Transitions:
//!   Closed → Open: failure_count >= threshold
//!   Open → HalfOpen: reset_timeout elapsed
//!   HalfOpen → Closed: probe succeeds
//!   HalfOpen → Open: probe fails

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use metrics_defs as counters;
use metrics_defs as gauges;

// ── State ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Closed => "closed",
            State::Open => "open",
            State::HalfOpen => "half_open",
        }
    }
}

// ── Config ───────────────────────────────────────────────

/// Configuration for a circuit breaker instance.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// How long to stay Open before transitioning to HalfOpen.
    pub reset_timeout: Duration,
    /// How long a successful streak must last to fully close after HalfOpen.
    /// (In this implementation, a single HalfOpen success closes immediately.)
    pub name: String,
}

impl CircuitBreakerConfig {
    pub fn new(name: &str, failure_threshold: u32, reset_timeout_secs: u64) -> Self {
        Self {
            failure_threshold,
            reset_timeout: Duration::from_secs(reset_timeout_secs),
            name: name.to_string(),
        }
    }
}

/// Preset configurations for common services.
impl CircuitBreakerConfig {
    pub fn ethereum_rpc() -> Self {
        Self::new("ethereum_rpc", 5, 30)
    }

    pub fn solana_rpc() -> Self {
        Self::new("solana_rpc", 5, 20)
    }

    pub fn wormhole_guardian() -> Self {
        Self::new("wormhole_guardian", 3, 60)
    }

    pub fn layerzero_api() -> Self {
        Self::new("layerzero_api", 3, 60)
    }

    pub fn price_oracle(source: &str) -> Self {
        Self::new(&format!("price_{source}"), 3, 15)
    }
}

// ── Inner state ──────────────────────────────────────────

struct Inner {
    state: State,
    failure_count: u32,
    last_failure: Option<Instant>,
    last_success: Option<Instant>,
    opened_at: Option<Instant>,
    total_rejections: u64,
    total_successes: u64,
    total_failures: u64,
}

// ── Circuit Breaker ──────────────────────────────────────

/// Thread-safe circuit breaker. Wraps any async call with failure detection
/// and automatic recovery.
#[derive(Clone)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    inner: Arc<Mutex<Inner>>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: Arc::new(Mutex::new(Inner {
                state: State::Closed,
                failure_count: 0,
                last_failure: None,
                last_success: None,
                opened_at: None,
                total_rejections: 0,
                total_successes: 0,
                total_failures: 0,
            })),
        }
    }

    /// Execute a fallible async operation through the circuit breaker.
    ///
    /// - Closed: execute normally, track failures
    /// - Open: reject immediately with CircuitOpen error
    /// - HalfOpen: allow one probe, transition based on result
    pub async fn call<F, T, E>(&self, operation: F) -> Result<T, CircuitError<E>>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        // Check state and possibly transition Open → HalfOpen
        {
            let mut inner = self.inner.lock().await;
            if inner.state == State::Open {
                if let Some(opened) = inner.opened_at {
                    if opened.elapsed() >= self.config.reset_timeout {
                        inner.state = State::HalfOpen;
                        tracing::info!(
                            breaker = %self.config.name,
                            "circuit_half_open"
                        );
                        self.update_gauge(&inner);
                    } else {
                        inner.total_rejections += 1;
                        counters::CIRCUIT_BREAKER_REJECTIONS
                            .with_label_values(&[&self.config.name])
                            .inc();
                        return Err(CircuitError::Open {
                            breaker: self.config.name.clone(),
                            remaining_secs: (self.config.reset_timeout - opened.elapsed())
                                .as_secs(),
                        });
                    }
                }
            }
        }

        // Execute the operation
        match operation.await {
            Ok(result) => {
                self.record_success().await;
                Ok(result)
            }
            Err(e) => {
                self.record_failure().await;
                Err(CircuitError::Inner(e))
            }
        }
    }

    /// Get the current state without side effects.
    pub async fn state(&self) -> State {
        let inner = self.inner.lock().await;
        inner.state
    }

    /// Get the breaker name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Manually reset to Closed (e.g., after config change).
    pub async fn reset(&self) {
        let mut inner = self.inner.lock().await;
        inner.state = State::Closed;
        inner.failure_count = 0;
        inner.opened_at = None;
        self.update_gauge(&inner);
        tracing::info!(breaker = %self.config.name, "circuit_manually_reset");
    }

    // ── Internal ─────────────────────────────────────

    async fn record_success(&self) {
        let mut inner = self.inner.lock().await;
        inner.last_success = Some(Instant::now());
        inner.total_successes += 1;

        match inner.state {
            State::HalfOpen => {
                // Probe succeeded → close the circuit
                inner.state = State::Closed;
                inner.failure_count = 0;
                inner.opened_at = None;
                tracing::info!(breaker = %self.config.name, "circuit_closed_after_recovery");
                counters::CIRCUIT_BREAKER_TRANSITIONS
                    .with_label_values(&[self.config.name.as_str(), "closed"])
                    .inc();
            }
            State::Closed => {
                // Reset consecutive failure count on success
                inner.failure_count = 0;
            }
            State::Open => {} // shouldn't happen
        }

        self.update_gauge(&inner);
    }

    async fn record_failure(&self) {
        let mut inner = self.inner.lock().await;
        inner.last_failure = Some(Instant::now());
        inner.failure_count += 1;
        inner.total_failures += 1;

        counters::CIRCUIT_BREAKER_FAILURES
            .with_label_values(&[&self.config.name])
            .inc();

        match inner.state {
            State::Closed => {
                if inner.failure_count >= self.config.failure_threshold {
                    inner.state = State::Open;
                    inner.opened_at = Some(Instant::now());
                    tracing::warn!(
                        breaker = %self.config.name,
                        failures = inner.failure_count,
                        threshold = self.config.failure_threshold,
                        reset_secs = self.config.reset_timeout.as_secs(),
                        "circuit_opened"
                    );
                    counters::CIRCUIT_BREAKER_TRANSITIONS
                        .with_label_values(&[self.config.name.as_str(), "open"])
                        .inc();
                }
            }
            State::HalfOpen => {
                // Probe failed → back to Open
                inner.state = State::Open;
                inner.opened_at = Some(Instant::now());
                tracing::warn!(
                    breaker = %self.config.name,
                    "circuit_reopened_after_failed_probe"
                );
                counters::CIRCUIT_BREAKER_TRANSITIONS
                    .with_label_values(&[self.config.name.as_str(), "open"])
                    .inc();
            }
            State::Open => {} // already open
        }

        self.update_gauge(&inner);
    }

    fn update_gauge(&self, inner: &Inner) {
        let val = match inner.state {
            State::Closed => 0,
            State::HalfOpen => 1,
            State::Open => 2,
        };
        gauges::CIRCUIT_BREAKER_STATE
            .with_label_values(&[&self.config.name])
            .set(val);
    }
}

// ── Error type ───────────────────────────────────────────

#[derive(Debug)]
pub enum CircuitError<E> {
    /// Circuit is open — call was rejected without executing.
    Open {
        breaker: String,
        remaining_secs: u64,
    },
    /// Call was executed but returned an error.
    Inner(E),
}

impl<E: std::fmt::Display> std::fmt::Display for CircuitError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitError::Open {
                breaker,
                remaining_secs,
            } => write!(
                f,
                "Circuit breaker '{breaker}' is open (resets in {remaining_secs}s)"
            ),
            CircuitError::Inner(e) => write!(f, "{e}"),
        }
    }
}

// ── Prometheus metrics ───────────────────────────────────
//
// Registered in the metrics module alongside existing counters/gauges.
// We define them here so they're co-located with the breaker logic.

pub mod metrics_defs {
    use once_cell::sync::Lazy;
    use prometheus::{IntCounterVec, IntGaugeVec, Opts};

    use crate::metrics::REGISTRY;

    /// How many times each breaker rejected a call (state=open).
    pub static CIRCUIT_BREAKER_REJECTIONS: Lazy<IntCounterVec> = Lazy::new(|| {
        let c = IntCounterVec::new(
            Opts::new("circuit_breaker_rejections_total", "Calls rejected by open circuit"),
            &["breaker"],
        )
        .unwrap();
        REGISTRY.register(Box::new(c.clone())).unwrap();
        c
    });

    /// Failure count per breaker (triggers opening).
    pub static CIRCUIT_BREAKER_FAILURES: Lazy<IntCounterVec> = Lazy::new(|| {
        let c = IntCounterVec::new(
            Opts::new("circuit_breaker_failures_total", "Failures recorded by circuit breaker"),
            &["breaker"],
        )
        .unwrap();
        REGISTRY.register(Box::new(c.clone())).unwrap();
        c
    });

    /// State transitions per breaker.
    pub static CIRCUIT_BREAKER_TRANSITIONS: Lazy<IntCounterVec> = Lazy::new(|| {
        let c = IntCounterVec::new(
            Opts::new("circuit_breaker_transitions_total", "Circuit breaker state transitions"),
            &["breaker", "to_state"],
        )
        .unwrap();
        REGISTRY.register(Box::new(c.clone())).unwrap();
        c
    });

    /// Current state per breaker: 0=closed, 1=half_open, 2=open.
    pub static CIRCUIT_BREAKER_STATE: Lazy<IntGaugeVec> = Lazy::new(|| {
        let g = IntGaugeVec::new(
            Opts::new("circuit_breaker_state", "Current circuit breaker state (0=closed, 1=half_open, 2=open)"),
            &["breaker"],
        )
        .unwrap();
        REGISTRY.register(Box::new(g.clone())).unwrap();
        g
    });
}


// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_breaker(threshold: u32, reset_ms: u64) -> CircuitBreaker {
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: threshold,
            reset_timeout: Duration::from_millis(reset_ms),
            name: "test".into(),
        })
    }

    #[tokio::test]
    async fn starts_closed() {
        let cb = test_breaker(3, 1000);
        assert_eq!(cb.state().await, State::Closed);
    }

    #[tokio::test]
    async fn success_stays_closed() {
        let cb = test_breaker(3, 1000);
        let result: Result<i32, CircuitError<String>> = cb.call(async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(cb.state().await, State::Closed);
    }

    #[tokio::test]
    async fn failures_below_threshold_stay_closed() {
        let cb = test_breaker(3, 1000);
        for _ in 0..2 {
            let _: Result<i32, _> = cb.call(async { Err::<i32, String>("fail".into()) }).await;
        }
        assert_eq!(cb.state().await, State::Closed);
    }

    #[tokio::test]
    async fn failures_at_threshold_opens() {
        let cb = test_breaker(3, 1000);
        for _ in 0..3 {
            let _: Result<i32, _> = cb.call(async { Err::<i32, String>("fail".into()) }).await;
        }
        assert_eq!(cb.state().await, State::Open);
    }

    #[tokio::test]
    async fn open_rejects_immediately() {
        let cb = test_breaker(1, 5000);
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("fail".into()) }).await;
        assert_eq!(cb.state().await, State::Open);

        let result: Result<i32, CircuitError<String>> = cb.call(async { Ok(99) }).await;
        assert!(matches!(result, Err(CircuitError::Open { .. })));
    }

    #[tokio::test]
    async fn open_to_half_open_after_timeout() {
        let cb = test_breaker(1, 50); // 50ms reset
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("fail".into()) }).await;
        assert_eq!(cb.state().await, State::Open);

        tokio::time::sleep(Duration::from_millis(60)).await;

        // Next call should transition to HalfOpen and execute
        let result: Result<i32, CircuitError<String>> = cb.call(async { Ok(1) }).await;
        assert_eq!(result.unwrap(), 1);
        assert_eq!(cb.state().await, State::Closed); // probe succeeded
    }

    #[tokio::test]
    async fn half_open_failure_reopens() {
        let cb = test_breaker(1, 50);
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("fail".into()) }).await;

        tokio::time::sleep(Duration::from_millis(60)).await;

        // Probe fails → back to Open
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("still broken".into()) }).await;
        assert_eq!(cb.state().await, State::Open);
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let cb = test_breaker(3, 1000);

        // 2 failures
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("f".into()) }).await;
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("f".into()) }).await;

        // 1 success resets count
        let _: Result<i32, _> = cb.call(async { Ok::<i32, String>(1) }).await;

        // 2 more failures should NOT open (count was reset)
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("f".into()) }).await;
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("f".into()) }).await;
        assert_eq!(cb.state().await, State::Closed);
    }

    #[tokio::test]
    async fn manual_reset() {
        let cb = test_breaker(1, 5000);
        let _: Result<i32, _> = cb.call(async { Err::<i32, String>("f".into()) }).await;
        assert_eq!(cb.state().await, State::Open);

        cb.reset().await;
        assert_eq!(cb.state().await, State::Closed);
    }

    #[tokio::test]
    async fn circuit_error_display() {
        let open = CircuitError::<String>::Open {
            breaker: "test".into(),
            remaining_secs: 25,
        };
        assert!(open.to_string().contains("open"));
        assert!(open.to_string().contains("25s"));

        let inner = CircuitError::Inner("rpc timeout".to_string());
        assert_eq!(inner.to_string(), "rpc timeout");
    }

    #[test]
    fn preset_configs() {
        let eth = CircuitBreakerConfig::ethereum_rpc();
        assert_eq!(eth.failure_threshold, 5);
        assert_eq!(eth.name, "ethereum_rpc");

        let sol = CircuitBreakerConfig::solana_rpc();
        assert_eq!(sol.failure_threshold, 5);

        let wh = CircuitBreakerConfig::wormhole_guardian();
        assert_eq!(wh.failure_threshold, 3);

        let lz = CircuitBreakerConfig::layerzero_api();
        assert_eq!(lz.failure_threshold, 3);

        let price = CircuitBreakerConfig::price_oracle("binance");
        assert_eq!(price.name, "price_binance");
    }
}
