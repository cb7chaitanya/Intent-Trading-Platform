# System Invariants

This document defines the correctness properties that must hold at all times in the IntentX system. Each invariant is a predicate over database and on-chain state that is true in every reachable state of a correct system. A violation means funds are at risk.

The invariant checker lives at `src/chaos/verify.rs`. The property-based tests live at `tests/invariant_proptest.rs`.

---

## Invariant Definitions

### INV-1: Fund Conservation

**Statement**: No funds are created or destroyed. For every asset type, the total balance held across all accounts equals the net of all ledger entries.

```
∀ asset ∈ {USDC, ETH, BTC, SOL}:
  Σ(available_balance + locked_balance) for all accounts
  =
  Σ(CREDIT amounts) − Σ(DEBIT amounts) in ledger_entries
```

**Verification SQL**:
```sql
SELECT b.asset, b.total, COALESCE(l.net, 0) as ledger,
       b.total - COALESCE(l.net, 0) as discrepancy
FROM (SELECT asset, SUM(available_balance + locked_balance) as total
      FROM balances GROUP BY asset) b
FULL JOIN (SELECT asset,
      SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net
      FROM ledger_entries GROUP BY asset) l
ON b.asset = l.asset
WHERE b.total != COALESCE(l.net, 0);
-- Must return 0 rows
```

**Scope**: Covers deposits, withdrawals, settlements, fees. Does NOT cover on-chain token balances (separate invariant — see INV-9).

**Known weakness**: `lock_balance()` and `unlock_balance()` do not create ledger entries. This is safe because they move amounts between `available` and `locked` within the same row — the SUM is unchanged. But it means the ledger cannot reconstruct the full history of lock/unlock operations.

**Checker**: `check_balance_sum_constant()` in `verify.rs:150-197`

---

### INV-2: No Double Settlement

**Statement**: Each fill is settled at most once. The `settle_fill()` transaction must be idempotent.

```
∀ intent_id:
  |{f ∈ fills : f.intent_id = intent_id ∧ f.settled = true}| ≤ expected
```

For market orders (single fill per intent): count must be ≤ 1.
For partial fills: each individual fill row has `settled` set at most once.

**Verification SQL**:
```sql
SELECT intent_id, COUNT(*) FROM fills
WHERE settled = true GROUP BY intent_id HAVING COUNT(*) > 1;
-- Must return 0 rows (for non-partial-fill intents)
```

**Protection mechanism**: `settle_fill()` uses `SELECT * FROM fills WHERE id = $1 FOR UPDATE`, then checks `fill.settled`. The row lock serializes concurrent attempts. The second caller sees `settled = true` and returns `AlreadySettled`.

**Failure mode**: Under `read uncommitted` isolation (not used, but hypothetically), two concurrent callers could both read `settled = false`. PostgreSQL default `read committed` prevents this because the second `FOR UPDATE` waits for the first transaction to commit.

**Checker**: `check_no_double_settlement()` in `verify.rs:240-263`

---

### INV-3: No Orphan Locked Funds

**Statement**: Every positive `locked_balance` is accounted for by at least one active intent.

```
∀ (account_id, asset) WHERE locked_balance > 0:
  ∃ intent WHERE intent.user_id = account.user_id
    ∧ intent.token_in = asset
    ∧ intent.status ∈ {open, bidding, matched, executing}
```

**Verification SQL**:
```sql
SELECT b.account_id, b.asset, b.locked_balance
FROM balances b WHERE b.locked_balance > 0
AND NOT EXISTS (
  SELECT 1 FROM intents i JOIN accounts a ON a.user_id::text = i.user_id
  WHERE a.id = b.account_id AND i.status IN ('open','bidding','matched','executing')
);
-- Must return 0 rows
```

**Failure modes**:
- Intent cancelled but `unlock_balance()` not called (process crash between status update and balance mutation)
- Intent expired by expiry worker but balance unlock fails
- Cross-chain settlement refunded but user's locked balance not released

**Checker**: `check_no_orphan_locked_funds()` in `verify.rs:205-233`

---

### INV-4: HTLC Atomicity

Four sub-invariants that together guarantee atomic swap correctness.

#### INV-4a: Secret-hash binding

```
∀ swap WHERE secret IS NOT NULL:
  SHA-256(swap.secret) = swap.secret_hash
```

If this is violated, a wrong preimage was accepted. The on-chain HTLC contract also verifies this, so a violation here means the off-chain DB was corrupted independently of the chain.

**Checker**: `check_htlc_secret_integrity()` in `verify.rs:301-345`. Computes SHA-256 in Rust and compares to stored hash.

#### INV-4b: Timelock enforcement

```
∀ swap WHERE status = 'refunded':
  swap.source_timelock < refund_timestamp
```

Refunds must not happen before the timelock expires. Enforced by `refund_swap()` which checks `Utc::now() < swap.source_timelock` and returns `InvalidState` if the timelock hasn't passed.

On-chain enforcement: the Anchor program's `refund()` instruction checks `Clock::get()?.unix_timestamp >= htlc.timelock`.

#### INV-4c: Mutual exclusion of claim and refund

```
∀ swap: ¬(status = 'source_unlocked' ∧ status = 'refunded')
```

Structurally impossible with a single enum column. But the stronger property is: once a swap reaches `dest_claimed`, it cannot be refunded. Enforced by:
- `refund_swap()` checks `swap.status != DestClaimed && swap.status != SourceUnlocked`
- SQL: `UPDATE ... WHERE status IN ('created', 'source_locked')` — excludes `dest_claimed`

#### INV-4d: Terminal convergence

```
∀ swap WHERE source_timelock < NOW():
  status ∈ {source_unlocked, refunded, expired, failed}
```

Every swap past its timelock must have reached a terminal state. The HTLC worker Phase 5 queries for expired non-terminal swaps and refunds them.

**Checker**: `check_htlc_terminal_states()` in `verify.rs:270-293`

---

### INV-5: Cross-Chain Leg Consistency

Three sub-invariants for cross-chain settlement legs.

#### INV-5a: Leg count

```
∀ fill_id ∈ cross_chain_legs:
  |{leg : leg.fill_id = fill_id}| = 2
```

Every cross-chain settlement has exactly one source leg (`leg_index=0`) and one dest leg (`leg_index=1`). Enforced by `UNIQUE (fill_id, leg_index)` DB constraint and `create_settlement()` which inserts exactly 2 rows.

#### INV-5b: Finalization

```
∀ (source, dest) WHERE source.status = 'confirmed' ∧ dest.status = 'confirmed':
  intent.status = 'completed'
```

Worker Phase 5 queries for fills where both legs are confirmed and the intent is not yet completed, then updates the intent. A violation means the finalization query failed or the worker crashed between querying and updating.

#### INV-5c: Refund cascade

```
∀ (source, dest) WHERE source.status = 'refunded':
  dest.status ∈ {refunded, failed, confirmed}
```

If the source leg is refunded, the dest leg must not be in an active state (`pending`, `escrowed`, `executing`). Worker Phase 4 cascades refunds: if a source leg (leg_index=0, status=escrowed) times out, it also refunds the dest leg.

**Checker**: `check_cross_chain_leg_consistency()` in `verify.rs:352-437`

---

### INV-6: No Negative Balances

```
∀ (account_id, asset):
  available_balance ≥ 0 ∧ locked_balance ≥ 0
```

**Verification SQL**:
```sql
SELECT account_id, asset, available_balance, locked_balance
FROM balances WHERE available_balance < 0 OR locked_balance < 0;
-- Must return 0 rows
```

**Not enforced by DB constraint.** The application checks `available >= amount` before deducting. A race condition between the check and the deduction could violate this. The only protection is the `SELECT ... FOR UPDATE` row lock in `settle_fill()` and `create_intent()`.

**Recommended fix**: `ALTER TABLE balances ADD CONSTRAINT chk_positive CHECK (available_balance >= 0 AND locked_balance >= 0)`. This makes the DB reject negative values regardless of application bugs.

**Checker**: `check_no_negative_balances()` in `verify.rs:120-143`

---

### INV-7: Ledger Double-Entry Balance

```
∀ (account_id, asset):
  available_balance + locked_balance
  =
  Σ(CREDIT amounts for this account+asset) − Σ(DEBIT amounts for this account+asset)
```

This is INV-1 at the per-account level rather than the global level. A violation means a specific account has a balance that doesn't match its own ledger history.

**Checker**: `check_ledger_debits_equal_credits()` in `verify.rs:440+`

---

### INV-8: Fill-HTLC Uniqueness

```
∀ fill_id:
  |{swap ∈ htlc_swaps : swap.fill_id = fill_id}| ≤ 1
```

Enforced by `UNIQUE (fill_id)` on `htlc_swaps`. A duplicate INSERT fails with a constraint violation.

---

### INV-9: On-Chain / Off-Chain Balance Agreement (not currently enforced)

```
∀ chain, vault_address:
  on_chain_token_balance(vault_address)
  ≥
  Σ(user balances on that chain that are backed by vault deposits)
```

This invariant bridges the on-chain and off-chain worlds. It is not currently verified by any automated checker because it requires querying chain state. A violation means the vault has been drained or the off-chain ledger overstates holdings.

---

## Property-Based Tests

### Design Principles

Each property test generates random operation sequences and verifies invariants hold after every step. The goal is to find edge cases that deterministic tests miss.

**Test structure**:
```
1. Spin up isolated Postgres (testcontainers)
2. Create test users and accounts
3. Execute N random operations from a set {deposit, withdraw, transfer, ...}
4. After every K operations, run invariant checks
5. Assert zero violations
```

### P-1: Conservation under random financial operations

**What**: Execute 200 random deposit/withdraw/transfer operations across 2 accounts, 4 assets. Check INV-1 and INV-6 every 20 operations.

**How to break it**: Remove the SQL transaction wrapper from `op_transfer()`. The debit completes but the credit does not if a crash occurs between them. The invariant checker detects the discrepancy.

**Implementation**: `tests/invariant_proptest.rs::prop_random_deposits_withdrawals_conserve_balance`

**What it guarantees**: No sequence of valid financial operations can create or destroy funds, regardless of order, amounts, or asset mix.

**What it does NOT guarantee**: Protection against concurrent operations (the test is single-threaded) or application code bugs (the test uses corrected atomic operations, not the actual `balances/service.rs` code which has known non-atomic paths).

---

### P-2: Settlement idempotency under concurrency

**What**: Create one fill. Spawn 20 concurrent tasks that all attempt `settle_fill()` simultaneously. Assert exactly 1 succeeds and INV-2 holds.

**How to break it**: Change PostgreSQL isolation level to `read uncommitted`. Both transactions read `settled = false` before either commits. Both proceed to debit/credit balances. Double settlement.

**Implementation**: `tests/invariant_proptest.rs::prop_concurrent_settlement_no_double`

**What it guarantees**: Under PostgreSQL `read committed` isolation (default), concurrent settlement of the same fill is safe. Exactly one caller wins; the rest see `AlreadySettled`.

---

### P-3: HTLC claim/refund mutual exclusion

**What**: Create an HTLC swap in `source_locked` with an expired timelock. Spawn two concurrent tasks: one attempts `claim` (status → `dest_claimed`), the other attempts `refund` (status → `refunded`). Assert exactly one succeeds.

**How to break it**: Remove the `AND status = 'source_locked'` clause from the claim UPDATE. Both updates succeed — the last one wins, and the system is in an inconsistent state where both claim and refund were applied.

**Implementation**: `tests/invariant_proptest.rs::prop_htlc_claim_refund_mutual_exclusion`

**What it guarantees**: The SQL WHERE clause on the status column provides mutual exclusion. The first UPDATE to commit changes the status; the second UPDATE matches 0 rows. INV-4c holds under concurrent access.

---

### P-4: Edge amounts preserve conservation

**What**: Test boundary values — deposit 0, deposit MAX, withdraw exact balance, withdraw more than balance. Check INV-1 and INV-6 after each.

**How to break it**: Allow `amount = 0` to create a ledger entry without changing the balance (or vice versa). The invariant checker sees a mismatch.

**Implementation**: `tests/invariant_proptest.rs::prop_zero_and_edge_amounts`

**What it guarantees**: Zero-amount and exact-balance operations do not violate conservation or produce negative balances.

---

### Proposed Tests (Not Yet Implemented)

#### P-5: Random intent lifecycle preserves no-orphan-locks

**What**: Generate 50 random intents with random amounts. Randomly cancel, settle, or let them expire. After all operations, check INV-3.

**How to break it**: Cancel an intent without calling `unlock_balance()`. The locked amount stays positive with no active intent.

**What it would guarantee**: Every code path that changes intent status also releases the corresponding balance lock.

#### P-6: Random cross-chain leg transitions preserve consistency

**What**: Create 20 cross-chain settlements. Randomly advance legs through Pending → Escrowed → Confirmed, or trigger timeouts. Check INV-5a/5b/5c after each step.

**How to break it**: Refund a source leg without cascading to the dest leg. INV-5c detects the orphaned dest leg.

#### P-7: Concurrent deposit + withdraw + settle does not violate conservation

**What**: Run 50 concurrent tasks performing random deposits, withdrawals, and settlements on the same accounts. Check INV-1 after all tasks complete.

**How to break it**: Use the actual `balances/service.rs` code (which has non-atomic `deposit()` and `transfer()`). A crash between the balance UPDATE and ledger INSERT creates a discrepancy.

**Why this matters**: P-1 tests corrected atomic operations. P-7 would test the actual production code and likely FIND violations — proving the non-atomic paths are real bugs, not theoretical risks.

#### P-8: Fuzz VAA parsing never panics

**What**: Generate 10,000 random byte arrays (0-1000 bytes). Pass each to `WormholeBridge::parse_vaa()`. Assert: either a valid `Vaa` is returned or an error is returned. Never a panic.

**How to break it**: Remove the length checks at the top of `parse_vaa()`. Array index out-of-bounds on short inputs.

---

## Invariant Verification Infrastructure

### Runtime checker (`src/chaos/verify.rs`)

Runs 8 checks against live database state. Used:
- After chaos test suite (via `run_with_pool()` in chaos engine)
- Manually via `InvariantChecker::new(&pool).run_all().await`
- In CI as a post-deployment gate

```rust
let report = InvariantChecker::new(&pool).run_all().await;
report.log();  // prints pass/fail report
assert!(report.passed());
```

### Integration tests (`tests/invariant_proptest.rs`)

4 property-based tests, gated behind `--features integration` (requires Docker for testcontainers).

```bash
cargo test --test invariant_proptest --features integration -- --nocapture
```

### Chaos test integration

The chaos engine (`src/chaos/engine.rs`) runs invariant checks automatically on shutdown when a `PgPool` is provided:

```rust
chaos::engine::run_with_pool(registry, schedule, cancel, Some(pool)).await;
// On cancellation: deactivates faults → runs InvariantChecker → logs report
```

This means: after every chaos test run, the invariant checker confirms that fault injection did not corrupt financial state.

---

## Threat Model

### What the invariants protect against

| Threat | Invariant | Protection |
|--------|-----------|-----------|
| Balance mutation without ledger trail | INV-1, INV-7 | Discrepancy detected by conservation check |
| Negative balance from concurrent access | INV-6 | Detected post-facto (no DB constraint yet) |
| Same fill settled twice | INV-2 | `SELECT FOR UPDATE` + `settled` flag |
| Locked funds with no active intent | INV-3 | Orphan detection query |
| Wrong HTLC preimage accepted | INV-4a | SHA-256 verification at 3 code points + on-chain |
| Refund before timelock | INV-4b | Application check + on-chain `Clock` check |
| Both HTLC claim and refund succeed | INV-4c | SQL WHERE clause mutual exclusion |
| Stuck HTLC past timelock | INV-4d | Worker Phase 5 + invariant detection |
| Cross-chain legs out of sync | INV-5 | Worker Phase 4 cascade + Phase 5 finalization |
| On-chain vault drained | INV-9 | Not currently enforced |

### What the invariants do NOT protect against

| Threat | Why not covered |
|--------|----------------|
| Compromised authority key (Solidity) | On-chain governance — not a DB invariant |
| Malicious solver bid | Risk engine rejects, but invariant doesn't verify price fairness |
| RPC returning stale data | Circuit breaker detects failures, but not stale correct-looking data |
| Clock skew between DB and chain | INV-4b uses DB time; chain uses its own clock |
| Zero-day in PostgreSQL | Invariants assume the DB executes SQL correctly |

---

## Summary Table

| ID | Invariant | Checker method | Property test | DB constraint |
|----|-----------|---------------|---------------|---------------|
| INV-1 | Fund conservation | `check_balance_sum_constant` | P-1 | None |
| INV-2 | No double settlement | `check_no_double_settlement` | P-2 | None (app-level FOR UPDATE) |
| INV-3 | No orphan locks | `check_no_orphan_locked_funds` | P-5 (proposed) | None |
| INV-4a | Secret-hash binding | `check_htlc_secret_integrity` | — | None (app-level SHA-256) |
| INV-4b | Timelock enforcement | — (app-level) | — | On-chain Clock check |
| INV-4c | Claim/refund exclusion | — (structural) | P-3 | Status enum column |
| INV-4d | Terminal convergence | `check_htlc_terminal_states` | — | None |
| INV-5a | Leg count = 2 | `check_cross_chain_leg_consistency` | P-6 (proposed) | `UNIQUE(fill_id, leg_index)` |
| INV-5b | Both confirmed → completed | `check_cross_chain_leg_consistency` | P-6 (proposed) | None |
| INV-5c | Refund cascade | `check_cross_chain_leg_consistency` | P-6 (proposed) | None |
| INV-6 | No negative balances | `check_no_negative_balances` | P-1, P-4 | **None (recommended: ADD CHECK)** |
| INV-7 | Per-account ledger match | `check_ledger_debits_equal_credits` | P-1 | None |
| INV-8 | Fill-HTLC uniqueness | — | — | `UNIQUE(fill_id)` |
| INV-9 | On-chain agreement | Not implemented | — | None |
