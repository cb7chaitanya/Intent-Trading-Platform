# Formal Verification Strategy: "Funds Are Never Lost or Duplicated"

## 1. System Invariants

### INV-1: Total Balance Conservation

**Statement**: For every asset type, the sum of all balances (available + locked)
across all accounts equals the sum of all deposits minus all withdrawals, as
recorded in the ledger.

```
∀ asset ∈ {USDC, ETH, BTC, SOL}:
  Σ(available_balance + locked_balance) WHERE asset = a
  ==
  Σ(ledger CREDIT WHERE asset = a) - Σ(ledger DEBIT WHERE asset = a)
```

**Database state**:
```sql
-- Must return 0 rows
SELECT b.asset, b.total, l.net
FROM (SELECT asset, SUM(available_balance + locked_balance) as total FROM balances GROUP BY asset) b
FULL JOIN (SELECT asset, SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net FROM ledger_entries GROUP BY asset) l
ON b.asset = l.asset
WHERE b.total != COALESCE(l.net, 0) OR l.net != COALESCE(b.total, 0)
```

**On-chain state**: For each chain, the platform's custodial wallet balance +
escrowed amounts in HTLC contracts + tokens in-flight across bridges should
equal the sum of all user balances for that chain's native assets.

**Violation scenarios**:
- Crash between `balance UPDATE` and `ledger INSERT` in deposit/withdraw
- `lock_balance()` / `unlock_balance()` create no ledger entries
- `transfer()` has two non-atomic UPDATEs (from debited, to never credited)

---

### INV-2: No Double Settlement

**Statement**: Each fill is settled at most once. No fill can transition from
`settled=false` to `settled=true` more than once.

```
∀ fill ∈ fills:
  COUNT(fills WHERE intent_id = fill.intent_id AND settled = TRUE) ≤ 1
```

**Database state**:
```sql
-- Must return 0 rows
SELECT intent_id, COUNT(*) FROM fills WHERE settled = TRUE
GROUP BY intent_id HAVING COUNT(*) > 1
```

**Violation scenarios**:
- Settlement worker processes same fill event twice (at-least-once delivery)
- `settle_fill()` idempotency check (`if fill.settled`) races with concurrent call
- Stream bus delivers duplicate `ExecutionCompleted` events after restart

**Current protection**: `settle_fill()` uses `SELECT ... FOR UPDATE` on the fill
row, then checks `fill.settled`. This is correct under serializable isolation
but can race under read-committed if two workers call simultaneously.

---

### INV-3: No Orphan Locked Funds

**Statement**: Every account with `locked_balance > 0` must have at least one
non-terminal intent that accounts for the lock. Locked funds without a
corresponding active intent are unreachable.

```
∀ (account, asset) WHERE locked_balance > 0:
  ∃ intent WHERE intent.user_id = account.user_id
    AND intent.status ∈ {Open, Bidding, Matched, Executing}
    AND intent.token_in = asset
```

**Database state**:
```sql
-- Must return 0 rows
SELECT b.account_id, b.asset, b.locked_balance
FROM balances b
WHERE b.locked_balance > 0
AND NOT EXISTS (
  SELECT 1 FROM intents i JOIN accounts a ON a.user_id::text = i.user_id
  WHERE a.id = b.account_id AND i.status IN ('open','bidding','matched','executing')
)
```

**Violation scenarios**:
- Intent cancelled/failed but `unlock_balance()` never called
- Process crash between intent status update and balance unlock
- Cross-chain leg timeout refunds leg but doesn't unlock user's balance

---

### INV-4: HTLC Correctness

**Sub-invariants**:

**INV-4a: Secret-hash binding**
```
∀ swap WHERE secret IS NOT NULL:
  SHA256(swap.secret) == swap.secret_hash
```

**INV-4b: Timelock enforcement**
```
∀ swap WHERE status = 'refunded':
  swap.source_timelock < refund_timestamp
```
(Cannot refund before timelock expires)

**INV-4c: Mutual exclusion**
```
∀ swap:
  NOT (status = 'source_unlocked' AND status = 'refunded')
```
(A swap is either claimed or refunded, never both)

**INV-4d: Terminal state convergence**
```
∀ swap WHERE source_timelock < NOW():
  status ∈ {source_unlocked, refunded, expired, failed}
```
(Every swap past its timelock must have reached a terminal state)

**INV-4e: Secret never stored before lock**
```
∀ swap WHERE status = 'created' AND source_lock_tx IS NULL:
  -- Secret may be stored (for worker retrieval) but must not be
  -- considered "revealed" — dest_claim_tx must be NULL
  swap.dest_claim_tx IS NULL
```

**Violation scenarios**:
- Wrong preimage accepted (hash mismatch)
- Refund executed before timelock (clock skew between DB and chain)
- Secret revealed on dest chain but DB not updated (crash after bridge call)
- Both claim and refund succeed on different chains due to timing

---

### INV-5: Cross-Chain Leg Consistency

**INV-5a: Leg count**
```
∀ fill_id in cross_chain_legs:
  COUNT(legs WHERE fill_id = f) == 2
```

**INV-5b: Finalization**
```
∀ (source, dest) WHERE source.status = 'confirmed' AND dest.status = 'confirmed':
  intent.status = 'completed'
```

**INV-5c: Refund cascade**
```
∀ (source, dest) WHERE source.status = 'refunded':
  dest.status ∈ {refunded, failed, confirmed}
  -- Dest cannot be in pending/escrowed/executing if source is refunded
```

**INV-5d: No leg without intent**
```
∀ leg ∈ cross_chain_legs:
  ∃ intent WHERE intent.id = leg.intent_id
```

---

### INV-6: No Negative Balances

```
∀ (account, asset):
  available_balance >= 0 AND locked_balance >= 0
```

**This should be enforced by CHECK constraint in DB but currently isn't.**

---

## 2. Failure Matrix

Each row is a failure scenario. Columns show which invariants are at risk and
what the expected system behavior should be.

### 2a. Infrastructure Failures

| # | Failure | When | INV at risk | Expected behavior | Current status |
|---|---------|------|-------------|-------------------|----------------|
| F1 | Postgres crash | During settle_fill() tx | INV-1 | Tx rolls back, retry from event | ✅ Atomic tx |
| F2 | Postgres crash | During deposit() | INV-1 | Balance updated, ledger missing | ❌ No tx boundary |
| F3 | Postgres crash | During transfer() | INV-1, INV-6 | From debited, to not credited | ❌ Two separate UPDATEs |
| F4 | Redis crash | During cache invalidation | None | Stale cache, eventually consistent | ⚠️ Acceptable |
| F5 | Worker crash | During lock_funds() callback | INV-3 | Source locked on-chain, DB still pending | ⚠️ Re-locks (idempotent?) |
| F6 | Worker crash | During verify_lock() callback | INV-5 | Re-verifies on next cycle | ✅ Idempotent |
| F7 | Worker crash | During release_funds() callback | INV-5 | Dest funds released, DB still pending | ❌ Duplicate release risk |

### 2b. Bridge / Chain Failures

| # | Failure | When | INV at risk | Expected behavior | Current status |
|---|---------|------|-------------|-------------------|----------------|
| B1 | Source tx reverts | After sending | INV-3 | Leg marked Failed, funds unlocked | ✅ Receipt check |
| B2 | Bridge timeout | VAA never arrives | INV-3, INV-5 | Timeout refund after 10min | ✅ Phase 4 |
| B3 | Dest chain tx reverts | During completeTransfer | INV-5 | Retry up to 5x, then fail | ✅ Retry logic |
| B4 | Chain reorg | After source confirmed | INV-1, INV-5 | Source leg confirmed prematurely | ❌ No reorg detection |
| B5 | Partial bridge failure | Source locked, dest fails | INV-1, INV-3 | Source locked forever if timeout broken | ⚠️ Timeout covers this |
| B6 | VAA with bad quorum | Forged/incomplete VAA | INV-1 | Reject VAA | ✅ verify_vaa() |
| B7 | Duplicate VAA submission | Same VAA submitted twice | INV-1 | Dest contract rejects | ✅ On-chain idempotency |

### 2c. Application Logic Failures

| # | Failure | When | INV at risk | Expected behavior | Current status |
|---|---------|------|-------------|-------------------|----------------|
| A1 | Duplicate settlement event | Stream redelivery | INV-2 | Idempotent settle_fill | ✅ FOR UPDATE + check |
| A2 | Concurrent settle_fill | Two workers same fill | INV-1, INV-2 | Serialized by row lock | ⚠️ Depends on isolation |
| A3 | Intent cancelled during settlement | Race | INV-3 | Fail settlement, unlock balance | ❌ No status check in settle |
| A4 | Solver crash mid-HTLC | After source lock | INV-4 | Timeout refund | ✅ Timelock covers this |
| A5 | Wrong secret accepted | Bug in verify | INV-4a | Hash check rejects | ✅ SHA-256 verify |
| A6 | Timeout refund + late claim | Race between claim and refund | INV-4c | Mutual exclusion via status check | ⚠️ No DB-level constraint |
| A7 | Cascading refund partial failure | Second refund_leg fails | INV-5c | Dest leg orphaned | ❌ Non-atomic cascade |

### 2d. Operational Failures

| # | Failure | When | INV at risk | Expected behavior | Current status |
|---|---------|------|-------------|-------------------|----------------|
| O1 | Deploy during active settlements | Rolling update | All | In-flight settlements finish or timeout | ⚠️ Graceful shutdown |
| O2 | Clock skew between DB and chain | HTLC timelock check | INV-4b | Premature or late refund | ❌ No clock sync check |
| O3 | RPC provider returns stale data | Balance query | INV-1 | Stale balance leads to over-lock | ⚠️ Circuit breaker helps |

---

## 3. Test Plan

### 3a. Property-Based Tests (QuickCheck-style)

These tests generate random sequences of operations and verify invariants hold
after each sequence.

**P1: Random deposit/withdraw sequences preserve conservation**
```
∀ sequence of (deposit, withdraw, lock, unlock, transfer) with random amounts:
  Σ(available + locked) == Σ(deposits) - Σ(withdrawals)
```
- Generate 100-1000 random operations
- After each operation, check INV-1
- Include edge cases: zero amounts, max amounts, same-account transfers

**P2: Random intent lifecycle preserves no-orphan-locks**
```
∀ sequence of (create_intent, cancel_intent, settle_fill) in random order:
  Every account with locked_balance > 0 has an active intent
```
- Generate random intents with random amounts
- Randomly cancel or settle them
- Check INV-3 after each step

**P3: Random HTLC lifecycle preserves correctness**
```
∀ sequence of (create_swap, lock, claim, refund) with random timing:
  INV-4a through INV-4e hold
```
- Generate swaps with random timelocks (some already expired)
- Randomly choose claim vs refund path
- Verify secret-hash binding, timelock enforcement, mutual exclusion

**P4: Concurrent settlement never doubles**
```
∀ N concurrent settle_fill() calls for the same fill:
  Exactly 1 succeeds, N-1 return AlreadySettled
  INV-2 holds
```
- Spawn 10-50 concurrent tasks all settling the same fill
- Assert exactly one succeeds

**P5: Random cross-chain leg transitions preserve consistency**
```
∀ sequence of (create_settlement, execute_source, confirm, fail, refund) in random order:
  INV-5a through INV-5d hold
```

### 3b. Deterministic Failure Injection Tests

These use the existing chaos framework to inject specific failures at specific
points and verify invariant recovery.

**D1: Crash after bridge.lock_funds() returns, before DB update**
- Inject WorkerCrash fault immediately after lock_funds returns
- Restart worker
- Assert: leg is re-processed, no duplicate on-chain lock (or duplicate is idempotent)
- Check: INV-1, INV-3

**D2: Crash after settle_fill() debit but before credit**
- This cannot happen because settle_fill uses atomic tx
- But verify: Kill process during settle_fill, restart, check INV-1

**D3: Crash during cascading refund**
- Inject fault between first and second refund_leg() in process_timeouts
- Restart worker
- Assert: second refund eventually happens in next cycle
- Check: INV-5c

**D4: Bridge returns error after partial delivery**
- Source chain locks funds
- Bridge verify_lock returns InTransit
- Bridge release_funds fails
- Assert: timeout eventually refunds source
- Check: INV-1, INV-3, INV-5

**D5: Concurrent claim and refund on HTLC**
- Set timelock to expire in 1 second
- Start claim process (slow bridge mock with 2s delay)
- Let refund trigger in parallel
- Assert: exactly one path completes, other is rejected
- Check: INV-4c

**D6: Chain reorg after source confirmation**
- Confirm source leg
- "Reorg" by setting source leg back to escrowed
- Assert: system doesn't finalize intent with unconfirmed source
- Check: INV-5b

### 3c. Fuzzing Scenarios

**FZ1: Random byte mutation on VAA**
- Generate valid VAA
- Randomly flip 1-5 bytes
- Pass to parse_vaa() and verify_vaa()
- Assert: either parses correctly with valid quorum, or returns error
- Never panics or produces invalid state

**FZ2: Random JSON mutation on guardian response**
- Generate valid guardian JSON response
- Randomly remove/modify fields
- Pass to fetch_vaa parsing
- Assert: returns error or valid Vaa, never panics

**FZ3: Random amount fuzzing on settlement**
- Generate fills with random amounts (0, 1, u64::MAX, negative via overflow)
- Run settle_fill()
- Assert: INV-1, INV-6 hold or operation is rejected

**FZ4: Random timing on HTLC operations**
- Create swap with random timelock (0s to 3600s)
- Randomly delay between operations (0ms to 5000ms)
- Execute full lifecycle or timeout
- Assert: INV-4 holds regardless of timing

**FZ5: Random concurrent operation mix**
- Spawn 50 tasks, each performing random operations:
  - Create intent, cancel intent, create fill, settle fill
  - Create HTLC swap, lock, claim, refund
  - Create cross-chain legs, confirm, fail, refund
- All against the same DB
- After all tasks complete, check all invariants

---

## 4. Invariant-to-Checker Mapping

Each invariant maps to a check in `src/chaos/verify.rs`:

| Invariant | Checker method | What it queries |
|-----------|---------------|-----------------|
| INV-1 | `check_balance_sum_constant()` | balances vs ledger_entries aggregates |
| INV-2 | `check_no_double_settlement()` | fills GROUP BY intent_id HAVING count > 1 |
| INV-3 | `check_no_orphan_locked_funds()` | balances LEFT JOIN intents |
| INV-4a | `check_htlc_secret_integrity()` | SHA256(secret) vs secret_hash for claimed swaps |
| INV-4d | `check_htlc_terminal_states()` | htlc_swaps WHERE timelock < NOW AND non-terminal |
| INV-5a | `check_cross_chain_leg_consistency()` | legs GROUP BY fill_id HAVING count != 2 |
| INV-5b | `check_cross_chain_leg_consistency()` | both confirmed but intent != completed |
| INV-5c | `check_cross_chain_leg_consistency()` | refund mismatch between source/dest |
| INV-6 | `check_no_negative_balances()` | balances WHERE available < 0 OR locked < 0 |
| — | `check_ledger_debits_equal_credits()` | per-account balance vs ledger |

---

## 5. Recommended Fixes (Priority Order)

### Critical (funds at risk)

1. **Add CHECK constraints to balances table**:
   ```sql
   ALTER TABLE balances ADD CONSTRAINT positive_available CHECK (available_balance >= 0);
   ALTER TABLE balances ADD CONSTRAINT positive_locked CHECK (locked_balance >= 0);
   ```

2. **Wrap transfer() in explicit transaction** (balances/service.rs):
   Two UPDATEs must be atomic or from-account loses funds on crash.

3. **Wrap deposit()/withdraw() balance+ledger in transaction** (balances/service.rs):
   Balance mutation and ledger entry must be atomic.

4. **Make cascading refund atomic** (cross_chain/worker.rs):
   Both leg refunds should happen in a single SQL transaction.

### High (correctness at risk)

5. **Add previous-status guard to leg transitions** (cross_chain/service.rs):
   `WHERE id = $1 AND status = $expected_previous_status`

6. **Add ledger entries for lock/unlock operations** (balances/service.rs):
   Without these, INV-1 cannot be verified for lock/unlock.

7. **Fix retry.rs record_fill_failure() SQL** (retry.rs:59):
   Generates duplicate UUIDs; needs separate id vs trade_id.

### Medium (operational risk)

8. **Add DB-level constraint for HTLC mutual exclusion**:
   ```sql
   ALTER TABLE htlc_swaps ADD CONSTRAINT htlc_no_claim_and_refund
     CHECK (NOT (status = 'source_unlocked' AND status = 'refunded'));
   ```
   (This is already structurally impossible with a single enum column but
   adding a trigger that prevents transitioning from dest_claimed to refunded
   would be stronger.)

9. **Add chain reorg detection**: Compare confirmed block number on re-check;
   if block number changed, reset leg to escrowed.

10. **Add clock skew guard to HTLC refund**: Compare chain timestamp (via
    eth_getBlockByNumber) with DB timestamp before refunding.
