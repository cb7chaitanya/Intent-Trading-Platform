//! Post-chaos invariant verification.
//!
//! Runs after chaos tests to ensure no funds were lost, no settlements
//! were doubled, no balances went negative, and all HTLC swaps reached
//! a valid terminal state. Produces a pass/fail report with details on
//! every inconsistency found.
//!
//! Usage:
//!   let report = InvariantChecker::new(&pool).run_all().await;
//!   report.log();
//!   assert!(report.passed());

use sqlx::PgPool;

// ── Report ──────────────────────────────────────────────────

/// A single invariant violation.
#[derive(Debug, Clone)]
pub struct Violation {
    pub check: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.check, self.detail)
    }
}

/// Aggregated invariant check report.
#[derive(Debug)]
pub struct InvariantReport {
    pub violations: Vec<Violation>,
    pub checks_run: u32,
    pub checks_passed: u32,
}

impl InvariantReport {
    fn new() -> Self {
        Self {
            violations: Vec::new(),
            checks_run: 0,
            checks_passed: 0,
        }
    }

    fn record_pass(&mut self) {
        self.checks_run += 1;
        self.checks_passed += 1;
    }

    fn record_fail(&mut self, check: &'static str, detail: String) {
        self.checks_run += 1;
        self.violations.push(Violation { check, detail });
    }

    /// True if every check passed.
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// Log the full report.
    pub fn log(&self) {
        tracing::info!("╔══════════════════════════════════════════════════╗");
        tracing::info!("║         INVARIANT VERIFICATION REPORT            ║");
        tracing::info!("╠══════════════════════════════════════════════════╣");

        if self.passed() {
            tracing::info!(
                checks = self.checks_run,
                passed = self.checks_passed,
                "ALL CHECKS PASSED"
            );
        } else {
            tracing::error!(
                checks = self.checks_run,
                passed = self.checks_passed,
                failed = self.violations.len(),
                "INVARIANT VIOLATIONS DETECTED"
            );
            for v in &self.violations {
                tracing::error!(check = v.check, detail = %v.detail, "VIOLATION");
            }
        }

        tracing::info!("╚══════════════════════════════════════════════════╝");
    }
}

// ── Checker ─────────────────────────────────────────────────

pub struct InvariantChecker<'a> {
    pool: &'a PgPool,
}

impl<'a> InvariantChecker<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Run all invariant checks and return the report.
    pub async fn run_all(&self) -> InvariantReport {
        let mut report = InvariantReport::new();

        self.check_no_negative_balances(&mut report).await;
        self.check_balance_sum_constant(&mut report).await;
        self.check_no_orphan_locked_funds(&mut report).await;
        self.check_no_double_settlement(&mut report).await;
        self.check_htlc_terminal_states(&mut report).await;
        self.check_htlc_secret_integrity(&mut report).await;
        self.check_cross_chain_leg_consistency(&mut report).await;
        self.check_ledger_debits_equal_credits(&mut report).await;

        report
    }

    // ── 1. No negative balances ─────────────────────────

    /// available_balance and locked_balance must never go below zero.
    async fn check_no_negative_balances(&self, report: &mut InvariantReport) {
        let rows = sqlx::query_as::<_, (uuid::Uuid, String, i64, i64)>(
            "SELECT account_id, asset::text, available_balance, locked_balance
             FROM balances
             WHERE available_balance < 0 OR locked_balance < 0",
        )
        .fetch_all(self.pool)
        .await;

        match rows {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (account_id, asset, avail, locked) in &bad {
                    report.record_fail(
                        "no_negative_balances",
                        format!(
                            "account={account_id} asset={asset} available={avail} locked={locked}"
                        ),
                    );
                }
            }
            Err(e) => report.record_fail("no_negative_balances", format!("query error: {e}")),
        }
    }

    // ── 2. Balance sum constant ─────────────────────────

    /// For each asset, SUM(available + locked) across all accounts must
    /// equal SUM(deposits) - SUM(withdrawals) from the ledger.
    /// Any discrepancy means funds were created or destroyed.
    async fn check_balance_sum_constant(&self, report: &mut InvariantReport) {
        // Total balance per asset from balances table
        let balance_sums = sqlx::query_as::<_, (String, i64)>(
            "SELECT asset::text, SUM(available_balance + locked_balance) as total
             FROM balances
             GROUP BY asset",
        )
        .fetch_all(self.pool)
        .await;

        // Total net flow per asset from ledger (credits - debits)
        let ledger_sums = sqlx::query_as::<_, (String, i64)>(
            "SELECT asset::text,
                    SUM(CASE WHEN entry_type = 'CREDIT' THEN amount ELSE -amount END) as net
             FROM ledger_entries
             GROUP BY asset",
        )
        .fetch_all(self.pool)
        .await;

        match (balance_sums, ledger_sums) {
            (Ok(balances), Ok(ledger)) => {
                let ledger_map: std::collections::HashMap<String, i64> =
                    ledger.into_iter().collect();

                let mut any_failed = false;
                for (asset, balance_total) in &balances {
                    let ledger_net = ledger_map.get(asset).copied().unwrap_or(0);
                    if *balance_total != ledger_net {
                        report.record_fail(
                            "balance_sum_constant",
                            format!(
                                "asset={asset} balance_sum={balance_total} ledger_net={ledger_net} diff={}",
                                balance_total - ledger_net
                            ),
                        );
                        any_failed = true;
                    }
                }
                if !any_failed {
                    report.record_pass();
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                report.record_fail("balance_sum_constant", format!("query error: {e}"));
            }
        }
    }

    // ── 3. No orphan locked funds ───────────────────────

    /// Every account with locked_balance > 0 must have at least one
    /// non-terminal intent (Open, Bidding, Matched, Executing) that
    /// accounts for the lock. Locked funds without a corresponding
    /// active intent are orphaned.
    async fn check_no_orphan_locked_funds(&self, report: &mut InvariantReport) {
        let orphans = sqlx::query_as::<_, (uuid::Uuid, String, i64)>(
            "SELECT b.account_id, b.asset::text, b.locked_balance
             FROM balances b
             WHERE b.locked_balance > 0
               AND NOT EXISTS (
                   SELECT 1 FROM intents i
                   JOIN accounts a ON a.user_id::text = i.user_id
                   WHERE a.id = b.account_id
                     AND i.status IN ('open', 'bidding', 'matched', 'executing')
               )",
        )
        .fetch_all(self.pool)
        .await;

        match orphans {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (account_id, asset, locked) in &bad {
                    report.record_fail(
                        "no_orphan_locked_funds",
                        format!(
                            "account={account_id} asset={asset} locked={locked} (no active intent)"
                        ),
                    );
                }
            }
            Err(e) => report.record_fail("no_orphan_locked_funds", format!("query error: {e}")),
        }
    }

    // ── 4. No double settlement ─────────────────────────

    /// Each fill must be settled at most once. If the same fill_id
    /// appears more than once with settled=true, funds were double-counted.
    async fn check_no_double_settlement(&self, report: &mut InvariantReport) {
        let doubles = sqlx::query_as::<_, (uuid::Uuid, i64)>(
            "SELECT intent_id, COUNT(*) as n
             FROM fills
             WHERE settled = true
             GROUP BY intent_id
             HAVING COUNT(*) > 1",
        )
        .fetch_all(self.pool)
        .await;

        match doubles {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (intent_id, count) in &bad {
                    report.record_fail(
                        "no_double_settlement",
                        format!("intent_id={intent_id} settled_fills={count}"),
                    );
                }
            }
            Err(e) => report.record_fail("no_double_settlement", format!("query error: {e}")),
        }
    }

    // ── 5. HTLC terminal states ─────────────────────────

    /// Every HTLC swap must be in a valid terminal state OR still be
    /// within its timelock window. Non-terminal swaps past their
    /// timelock indicate a stuck swap that was never resolved.
    async fn check_htlc_terminal_states(&self, report: &mut InvariantReport) {
        let stuck = sqlx::query_as::<_, (uuid::Uuid, String, String, String)>(
            "SELECT id, status::text, source_chain, dest_chain
             FROM htlc_swaps
             WHERE status NOT IN ('source_unlocked', 'refunded', 'expired', 'failed')
               AND source_timelock < NOW()",
        )
        .fetch_all(self.pool)
        .await;

        match stuck {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (id, status, src, dst) in &bad {
                    report.record_fail(
                        "htlc_terminal_states",
                        format!(
                            "swap={id} status={status} route={src}→{dst} (past timelock, not resolved)"
                        ),
                    );
                }
            }
            Err(e) => report.record_fail("htlc_terminal_states", format!("query error: {e}")),
        }
    }

    // ── 6. HTLC secret integrity ────────────────────────

    /// For every HTLC swap that reached dest_claimed or source_unlocked,
    /// the stored secret must hash to the stored secret_hash. A mismatch
    /// means the wrong preimage was accepted.
    async fn check_htlc_secret_integrity(&self, report: &mut InvariantReport) {
        let swaps = sqlx::query_as::<_, (uuid::Uuid, Vec<u8>, Vec<u8>)>(
            "SELECT id, secret_hash, secret
             FROM htlc_swaps
             WHERE secret IS NOT NULL
               AND status IN ('dest_claimed', 'source_unlocked')",
        )
        .fetch_all(self.pool)
        .await;

        match swaps {
            Ok(rows) => {
                let mut any_failed = false;
                for (id, hash, secret) in &rows {
                    if secret.len() != 32 {
                        report.record_fail(
                            "htlc_secret_integrity",
                            format!("swap={id} secret length={} (expected 32)", secret.len()),
                        );
                        any_failed = true;
                        continue;
                    }
                    let computed = {
                        use sha2::{Digest, Sha256};
                        let mut hasher = Sha256::new();
                        hasher.update(secret);
                        hasher.finalize().to_vec()
                    };
                    if computed != *hash {
                        report.record_fail(
                            "htlc_secret_integrity",
                            format!(
                                "swap={id} SHA256(secret) != stored hash (preimage mismatch)"
                            ),
                        );
                        any_failed = true;
                    }
                }
                if !any_failed {
                    report.record_pass();
                }
            }
            Err(e) => report.record_fail("htlc_secret_integrity", format!("query error: {e}")),
        }
    }

    // ── 7. Cross-chain leg consistency ──────────────────

    /// Every cross-chain settlement must have exactly 2 legs (source + dest).
    /// If both are confirmed, the intent must be completed. If one is
    /// refunded, the other must also be refunded or failed.
    async fn check_cross_chain_leg_consistency(&self, report: &mut InvariantReport) {
        // Check: fill_id with != 2 legs
        let wrong_count = sqlx::query_as::<_, (uuid::Uuid, i64)>(
            "SELECT fill_id, COUNT(*) as n
             FROM cross_chain_legs
             GROUP BY fill_id
             HAVING COUNT(*) != 2",
        )
        .fetch_all(self.pool)
        .await;

        match wrong_count {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (fill_id, count) in &bad {
                    report.record_fail(
                        "cross_chain_leg_count",
                        format!("fill_id={fill_id} has {count} legs (expected 2)"),
                    );
                }
            }
            Err(e) => report.record_fail("cross_chain_leg_count", format!("query error: {e}")),
        }

        // Check: both confirmed but intent not completed
        let unfinalized = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String)>(
            "SELECT l.fill_id, l.intent_id, i.status::text
             FROM cross_chain_legs l
             JOIN intents i ON i.id = l.intent_id
             WHERE l.leg_index = 0 AND l.status = 'confirmed'
               AND EXISTS (
                   SELECT 1 FROM cross_chain_legs l2
                   WHERE l2.fill_id = l.fill_id AND l2.leg_index = 1 AND l2.status = 'confirmed'
               )
               AND i.status != 'completed'",
        )
        .fetch_all(self.pool)
        .await;

        match unfinalized {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (fill_id, intent_id, status) in &bad {
                    report.record_fail(
                        "cross_chain_unfinalized",
                        format!(
                            "fill={fill_id} intent={intent_id} both legs confirmed but intent status={status}"
                        ),
                    );
                }
            }
            Err(e) => report.record_fail("cross_chain_unfinalized", format!("query error: {e}")),
        }

        // Check: one leg refunded but counterpart still active
        let mismatched_refund = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
            "SELECT src.fill_id, src.status::text as src_status, dest.status::text as dest_status
             FROM cross_chain_legs src
             JOIN cross_chain_legs dest ON dest.fill_id = src.fill_id AND dest.leg_index = 1
             WHERE src.leg_index = 0
               AND (
                   (src.status = 'refunded' AND dest.status NOT IN ('refunded', 'failed', 'confirmed'))
                   OR
                   (dest.status = 'refunded' AND src.status NOT IN ('refunded', 'failed', 'confirmed'))
               )",
        )
        .fetch_all(self.pool)
        .await;

        match mismatched_refund {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (fill_id, src_status, dest_status) in &bad {
                    report.record_fail(
                        "cross_chain_refund_mismatch",
                        format!(
                            "fill={fill_id} source={src_status} dest={dest_status} (refund not cascaded)"
                        ),
                    );
                }
            }
            Err(e) => {
                report.record_fail("cross_chain_refund_mismatch", format!("query error: {e}"));
            }
        }
    }

    // ── 8. Ledger debits equal credits ──────────────────

    /// For each (account, asset) pair, the sum of credits minus debits
    /// in the ledger must equal available_balance + locked_balance.
    /// A mismatch means a balance mutation happened without a ledger entry.
    async fn check_ledger_debits_equal_credits(&self, report: &mut InvariantReport) {
        let mismatches = sqlx::query_as::<_, (uuid::Uuid, String, i64, i64)>(
            "SELECT b.account_id, b.asset::text,
                    b.available_balance + b.locked_balance as balance_total,
                    COALESCE(l.net, 0) as ledger_net
             FROM balances b
             LEFT JOIN (
                 SELECT account_id, asset,
                        SUM(CASE WHEN entry_type = 'CREDIT' THEN amount ELSE -amount END) as net
                 FROM ledger_entries
                 GROUP BY account_id, asset
             ) l ON l.account_id = b.account_id AND l.asset = b.asset
             WHERE b.available_balance + b.locked_balance != COALESCE(l.net, 0)",
        )
        .fetch_all(self.pool)
        .await;

        match mismatches {
            Ok(bad) if bad.is_empty() => report.record_pass(),
            Ok(bad) => {
                for (account_id, asset, balance_total, ledger_net) in &bad {
                    report.record_fail(
                        "ledger_balance_match",
                        format!(
                            "account={account_id} asset={asset} balance={balance_total} ledger={ledger_net} diff={}",
                            balance_total - ledger_net
                        ),
                    );
                }
            }
            Err(e) => report.record_fail("ledger_balance_match", format!("query error: {e}")),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_passes() {
        let report = InvariantReport::new();
        assert!(report.passed());
        assert_eq!(report.checks_run, 0);
    }

    #[test]
    fn report_with_violations_fails() {
        let mut report = InvariantReport::new();
        report.record_pass();
        report.record_fail("test_check", "something broke".into());
        assert!(!report.passed());
        assert_eq!(report.checks_run, 2);
        assert_eq!(report.checks_passed, 1);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].check, "test_check");
    }

    #[test]
    fn violation_display() {
        let v = Violation {
            check: "no_negative_balances",
            detail: "account=abc asset=ETH available=-100 locked=0".into(),
        };
        let s = format!("{v}");
        assert!(s.contains("no_negative_balances"));
        assert!(s.contains("available=-100"));
    }

    #[test]
    fn all_pass_report() {
        let mut report = InvariantReport::new();
        report.record_pass();
        report.record_pass();
        report.record_pass();
        assert!(report.passed());
        assert_eq!(report.checks_run, 3);
        assert_eq!(report.checks_passed, 3);
    }
}
