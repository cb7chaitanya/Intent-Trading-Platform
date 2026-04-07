# Database Schema Reference

PostgreSQL 16. 39 migrations. 5 partitioned tables. 12 custom enum types.

---

## 1. Entity-Relationship Diagram

```
┌──────────┐
│  users   │
│──────────│
│ id (PK)  │
│ email (U)│
│ pw_hash  │
└────┬─────┘
     │ 1
     │
     │ N
┌────▼──────┐         ┌──────────────┐
│ accounts  │         │   roles      │
│───────────│         │──────────────│
│ id (PK)   │         │ id (PK)      │
│ user_id(FK│    ┌────│ name (U)     │
│ type      │    │    └──────┬───────┘
└────┬──────┘    │           │
     │ 1         │           │ N
     │           │    ┌──────▼───────┐
     │      ┌────▼────▼──┐   │permissions│
     │      │ user_roles │   │───────────│
     │      │────────────│   │ role_id(FK│
     │      │user_id(FK) │   │ resource  │
     │      │role_id(FK) │   │ action    │
     │      └────────────┘   └───────────┘
     │
     ├──────────────────────────────────────┐
     │ N                                    │ N
┌────▼──────┐                        ┌──────▼───────┐
│ balances  │                        │  api_keys    │
│───────────│                        │──────────────│
│ id (PK)   │                        │ id (PK)      │
│account(FK)│                        │ user_id (FK) │
│ asset     │                        │ key_hash     │
│ available │                        │ key_prefix   │
│ locked    │                        │ permissions[]│
│───────────│                        │ revoked      │
│U(acct,ast)│                        └──────────────┘
└────┬──────┘
     │
     │ referenced by
     │
┌────▼───────────┐
│ ledger_entries │  (partitioned by created_at)
│────────────────│
│ id             │
│ account_id(FK) │
│ asset          │
│ amount         │
│ entry_type     │  DEBIT | CREDIT
│ reference_type │  TRADE | DEPOSIT | WITHDRAWAL | FEE
│ reference_id   │
│ created_at     │
│────────────────│
│PK(id,created_at│
└────────────────┘


┌──────────────────────────────────────────────────────────────────────┐
│                         TRADING FLOW                                 │
└──────────────────────────────────────────────────────────────────────┘

┌────────────┐     ┌─────────────┐     ┌──────────────┐
│  intents   │ 1─N │    bids     │     │  executions  │
│────────────│     │─────────────│     │──────────────│
│ id (PK)    │◀────│ intent_id   │     │ intent_id    │
│ user_id    │     │ solver_id   │     │ solver_id    │
│ token_in   │     │ amount_out  │     │ tx_hash      │
│ token_out  │     │ fee         │     │ status       │
│ amount_in  │     └─────────────┘     └──────────────┘
│min_amount  │
│ deadline   │     ┌─────────────┐     ┌──────────────┐
│ status     │ 1─N │   fills     │ 1─N │ transactions │
│ order_type │◀────│─────────────│◀────│──────────────│
│ limit_price│     │ intent_id   │     │ fill_id (FK) │
│ stop_price │     │ solver_id   │     │ tx_hash      │
│source_chain│     │ price       │     │ chain        │
│ dest_chain │     │ qty         │     │ status       │
│ cross_chain│     │ filled_qty  │     │ gas_used     │
└─────┬──────┘     │ settled     │     │ block_number │
      │            │ settled_at  │     │ confirmations│
      │            └──────┬──────┘     └──────────────┘
      │                   │
      │                   │ fill_id
      │                   │
      ├───────────────────┼─────────────────────────────┐
      │                   │                             │
      │ 1─2              │ 1                           │ 1
┌─────▼──────────┐ ┌─────▼──────────┐         ┌───────▼────────┐
│cross_chain_legs│ │  htlc_swaps    │         │failed_settlemts│
│────────────────│ │────────────────│         │────────────────│
│ id (PK)        │ │ id (PK)        │         │ id (PK)        │
│ intent_id (FK) │ │ fill_id (U)    │         │ trade_id (U,FK)│
│ fill_id        │ │ intent_id (FK) │         │ fill_id        │
│ leg_index      │ │ secret_hash    │         │ retry_count    │
│ chain          │ │ secret         │         │ last_error     │
│ from_address   │ │ source_chain   │         │ next_retry_at  │
│ to_address     │ │ source_sender  │         │ perm_failed    │
│ token_mint     │ │ source_receiver│         └────────────────┘
│ amount         │ │ source_amount  │
│ tx_hash        │ │ source_timelock│
│ status         │ │ dest_chain     │
│ error          │ │ dest_sender    │
│ timeout_at     │ │ dest_amount    │
│ confirmed_at   │ │ status         │
│────────────────│ │ solver_id      │
│U(fill,leg_idx) │ │ error          │
└────────────────┘ │ locked_at      │
                   │ claimed_at     │
                   │ completed_at   │
                   └────────────────┘


┌──────────────────────────────────────────────────────────────────────┐
│                           TWAP FLOW                                  │
└──────────────────────────────────────────────────────────────────────┘

┌───────────────┐     ┌───────────────────┐
│ twap_intents  │ 1─N │twap_child_intents │
│───────────────│     │───────────────────│
│ id (PK)       │◀────│ twap_id (FK)      │
│ user_id       │     │ intent_id         │──── points to intents.id
│ account_id    │     │ slice_index       │     (nil UUID until submitted)
│ token_in      │     │ qty               │
│ token_out     │     │ status            │
│ total_qty     │     │ scheduled_at      │
│ filled_qty    │     └───────────────────┘
│ slices_total  │
│slices_complete│
│ status        │
└───────────────┘


┌──────────────────────────────────────────────────────────────────────┐
│                       MARKET DATA                                    │
└──────────────────────────────────────────────────────────────────────┘

┌────────────┐     ┌───────────────┐     ┌──────────────┐
│  markets   │ 1─N │ market_trades │     │market_prices │
│────────────│     │───────────────│     │──────────────│
│ id (PK)    │◀────│ market_id     │     │ market_id(PK)│
│ base_asset │     │ buyer_acct    │     │ price        │
│ quote_asset│     │ seller_acct   │     │ source       │
│ tick_size  │     │ price         │     │ updated_at   │
│min_order_sz│     │ qty, fee      │     └──────────────┘
│ fee_rate   │     └───────────────┘
│ chain      │
│U(base,quot)│
└────────────┘

┌────────────┐
│  wallets   │
│────────────│
│ id (PK)    │
│account_id  │
│address (U) │
│ chain      │
│encrypt_key │
│ nonce      │
│ active     │
└────────────┘

┌────────────┐
│  solvers   │
│────────────│
│ id (PK)    │
│ name       │
│ active     │
│ rep_score  │
│total_fills │
│failed_fills│
└────────────┘
```

---

## 2. Data Flow: Intent Lifecycle

### Single-chain intent

```
Step  Table mutation                        Who                 Notes
────  ────────────────────────────────────  ─────────────       ─────
1     INSERT intents (status=open)          IntentService       amount_in locked in step 2
2     UPDATE balances: avail -= amount,     IntentService       Inside same transaction as step 1
        locked += amount                                        (SELECT FOR UPDATE on balance row)
3     INSERT ledger_entries (not done)      —                   Lock/unlock don't create ledger entries
4     INSERT bids (per solver)              BidService          Multiple solvers bid during auction
5     INSERT fills (settled=false)          ExecutionEngine     Best bid → fill
6     INSERT executions (status=pending)    ExecutionEngine     Links intent → solver → tx
7     settle_fill (atomic transaction):     SettlementEngine
        UPDATE balances (6 mutations)                           Unlock buyer, credit seller, fees
        INSERT ledger_entries (4-6 rows)                        Double-entry for each movement
        UPDATE fills SET settled=true                           Marks fill as processed
8     UPDATE intents SET status=completed   SettlementWorker    After all fills settled
```

### Cross-chain intent

```
Step  Table mutation                        Who                 Notes
────  ────────────────────────────────────  ─────────────       ─────
1-6   Same as single-chain                  Same                Intent has cross_chain=true
7     INSERT cross_chain_legs ×2            CrossChainService   source(leg_index=0) + dest(leg_index=1)
        Both status=pending                                     timeout_at = NOW() + 600s
8     UPDATE leg SET status=escrowed,       Worker Phase 1      After bridge.lock_funds()
        tx_hash=source_tx
9     UPDATE leg SET status=confirmed       Worker Phase 2      After bridge.verify_lock()
10    UPDATE leg SET status=executing,      Worker Phase 3      After bridge.release_funds()
        tx_hash=dest_tx
11    UPDATE intents SET status=completed   Worker Phase 5      When both legs confirmed
```

### HTLC swap

```
Step  Table mutation                        Who                 Notes
────  ────────────────────────────────────  ─────────────       ─────
1     INSERT htlc_swaps (status=created)    HtlcService         secret_hash stored, secret=NULL
2     UPDATE htlc_swaps SET secret=$S       HtlcService         store_secret() verifies SHA-256(S)==H
3     UPDATE SET status=source_locked,      Worker Phase 1      After bridge.lock_funds()
        source_lock_tx, locked_at
4     UPDATE SET dest_lock_tx              Worker Phase 2      Solver's mirror HTLC observed
5     UPDATE SET status=dest_claimed,       Worker Phase 3      Secret revealed on dest chain
        secret=$S, dest_claim_tx, claimed_at
6     UPDATE SET status=source_unlocked,    Worker Phase 4      Solver uses S on source chain
        source_unlock_tx, completed_at
```

---

## 3. Invariants

### INV-1: Balance conservation

```
For every asset type:
  SUM(available_balance + locked_balance) across ALL accounts
  ==
  SUM(CREDIT amounts) - SUM(DEBIT amounts) in ledger_entries
```

**Verification query:**
```sql
SELECT b.asset, b.total as balance_sum, COALESCE(l.net, 0) as ledger_net,
       b.total - COALESCE(l.net, 0) as discrepancy
FROM (SELECT asset, SUM(available_balance + locked_balance) as total
      FROM balances GROUP BY asset) b
FULL JOIN (SELECT asset,
      SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net
      FROM ledger_entries GROUP BY asset) l
ON b.asset = l.asset
WHERE b.total != COALESCE(l.net, 0);
```

**Must return 0 rows.** Any discrepancy means funds were created or destroyed.

**Known gap**: `lock_balance()` and `unlock_balance()` in `balances/service.rs` do NOT create ledger entries. Only `deposit()`, `withdraw()`, and `settle_fill()` create them. This means the ledger tracks net deposits/withdrawals + settlements, but not the lock/unlock movements. The invariant still holds because lock/unlock are purely internal (they move between `available` and `locked` within the same row, so the SUM doesn't change).

### INV-2: No negative balances

```
For every (account_id, asset):
  available_balance >= 0
  locked_balance >= 0
```

**Verification query:**
```sql
SELECT account_id, asset, available_balance, locked_balance
FROM balances
WHERE available_balance < 0 OR locked_balance < 0;
```

**Must return 0 rows.**

**No CHECK constraint exists in the schema.** The application enforces this via balance checks before mutations. A concurrent race or crash can violate this. Recommended fix: `ALTER TABLE balances ADD CONSTRAINT positive_available CHECK (available_balance >= 0)`.

### INV-3: No double settlement

```
For every intent_id:
  COUNT(fills WHERE intent_id = X AND settled = true) should be consistent
  with the settlement model (one fill per intent for market orders,
  multiple for partial fills but each settled at most once)
```

**Verification query:**
```sql
SELECT intent_id, COUNT(*) as settled_fills
FROM fills WHERE settled = true
GROUP BY intent_id
HAVING COUNT(*) > 1;
```

For non-partial-fill intents, this must return 0 rows.

**Protection**: `settle_fill()` uses `SELECT ... FOR UPDATE` on the fill row and checks `fill.settled` before proceeding. This serializes concurrent settlement attempts.

### INV-4: Cross-chain leg consistency

```
For every fill_id in cross_chain_legs:
  COUNT(legs) == 2                                    (one source, one dest)
  If both status = 'confirmed': intent must be 'completed'
  If source status = 'refunded': dest NOT IN ('pending', 'escrowed', 'executing')
```

**Verification query:**
```sql
-- Wrong leg count
SELECT fill_id, COUNT(*) FROM cross_chain_legs GROUP BY fill_id HAVING COUNT(*) != 2;

-- Both confirmed but intent not completed
SELECT l.fill_id, l.intent_id, i.status
FROM cross_chain_legs l JOIN intents i ON i.id = l.intent_id
WHERE l.leg_index = 0 AND l.status = 'confirmed'
AND EXISTS (SELECT 1 FROM cross_chain_legs l2
  WHERE l2.fill_id = l.fill_id AND l2.leg_index = 1 AND l2.status = 'confirmed')
AND i.status != 'completed';

-- Refund not cascaded
SELECT s.fill_id, s.status as source, d.status as dest
FROM cross_chain_legs s JOIN cross_chain_legs d ON d.fill_id = s.fill_id AND d.leg_index = 1
WHERE s.leg_index = 0
AND s.status = 'refunded' AND d.status NOT IN ('refunded', 'failed', 'confirmed');
```

All three queries must return 0 rows.

### INV-5: HTLC secret-hash binding

```
For every htlc_swap WHERE secret IS NOT NULL:
  SHA-256(secret) == secret_hash
```

This cannot be verified in pure SQL (no native SHA-256 function without pgcrypto). The invariant checker in `src/chaos/verify.rs` performs this in Rust.

### INV-6: Unique fill per HTLC

```
UNIQUE (fill_id) on htlc_swaps
UNIQUE (fill_id, leg_index) on cross_chain_legs
```

Enforced by DB constraints. INSERT with a duplicate will fail with a unique violation.

---

## 4. Failure Scenarios

### 4.1 Partial write: crash between balance update and ledger insert

**Where**: `balances/service.rs` `deposit()` and `withdraw()`. The balance UPDATE and ledger INSERT are **not** in an explicit transaction.

**Effect**: Balance changes but ledger is missing the entry. INV-1 is violated — the balance sum no longer equals the ledger sum.

**Detection**: The conservation query in INV-1 will show a discrepancy for the affected asset.

**Recovery**: Insert the missing ledger entry manually based on the balance's `updated_at` timestamp and the last known good state.

**Prevention**: Wrap both operations in an explicit SQL transaction.

### 4.2 Partial write: crash between two balance updates in transfer()

**Where**: `balances/service.rs` `transfer()`. Two separate UPDATEs for `from` and `to` accounts.

**Effect**: `from` account debited, `to` account never credited. Funds destroyed.

**Detection**: INV-1 shows discrepancy. INV-2 may show negative `from` balance if the debit exceeded available.

**Prevention**: Wrap both UPDATEs in a single transaction (same fix as 4.1).

### 4.3 Worker crash during cross-chain lock

**Where**: Cross-chain worker Phase 1. `bridge.lock_funds()` returns success, but the worker crashes before executing `UPDATE cross_chain_legs SET status = 'escrowed'`.

**Effect**: Funds locked on-chain, but DB still shows `pending`. On restart, Phase 1 picks up the leg again and calls `lock_funds()` a second time, creating a duplicate on-chain lock.

**Detection**: Two on-chain transactions from the same sender to the same Token Bridge for the same amount within the timeout window.

**Recovery**: The timeout refund (Phase 4) will eventually refund both locks after `timeout_at` passes (if the bridge contract supports it).

### 4.4 Race condition: concurrent settle_fill()

**Where**: `settlement/engine.rs`. Two workers or two events trigger `settle_fill()` for the same fill.

**Effect**: The `SELECT ... FOR UPDATE` serializes access. The first caller sets `settled = true`. The second caller reads `settled = true` and returns `AlreadySettled`. No fund duplication.

**Requirement**: This only works under PostgreSQL's default `read committed` isolation. Under `read uncommitted`, both callers could read `settled = false` simultaneously.

### 4.5 Race condition: HTLC claim vs refund

**Where**: `htlc/service.rs`. Claim uses `WHERE status = 'source_locked'`. Refund uses `WHERE status IN ('created', 'source_locked')`.

**Effect**: Both target `source_locked`. Only one UPDATE can match — whichever commits first changes the status, and the second UPDATE affects 0 rows. Mutual exclusion is guaranteed by the single-column enum status.

**Not guaranteed**: If two concurrent transactions both read `status = 'source_locked'` before either commits, both UPDATEs will execute. PostgreSQL's default `read committed` isolation handles this correctly — the second UPDATE will see the first's committed change and affect 0 rows. But this is a runtime guarantee, not a schema constraint.

### 4.6 Cascading refund partial failure

**Where**: `cross_chain/worker.rs` Phase 4. Source leg refunded (line 196), then dest leg refund attempted (line 205). If the second call fails, the source is refunded but dest is not.

**Effect**: Inconsistent state. Source refunded, dest orphaned in `pending` or `executing`.

**Detection**: INV-4 refund cascade query catches this.

**Recovery**: Next Phase 4 cycle picks up the dest leg independently (its `timeout_at` has also passed) and refunds it.

---

## 5. Partitioning Strategy

Five tables are partitioned by RANGE on `created_at` (or `created_ts`):

| Table | Partition key | Retention |
|-------|--------------|-----------|
| `trades` | `created_at` | 6 months (configurable via `partition_retention_months`) |
| `ledger_entries` | `created_at` | 6 months |
| `fills` | `created_at` | 6 months |
| `executions` | `created_ts` | 6 months |
| `market_trades` | `created_at` | 6 months |

Partitions are created monthly. The `partition_archival` worker runs hourly and drops partitions older than the retention window.

**Consequence**: Queries that span > 6 months of historical data will fail with "no partition" errors. Archival data must be restored from backups if needed.

---

## 6. Indexing Strategy

### Hot-path indexes (settlement and worker queries)

| Table | Index | Purpose |
|-------|-------|---------|
| `balances` | `(account_id, asset) UNIQUE` | Balance lookup during settlement |
| `fills` | `(intent_id) WHERE settled = FALSE` | Find unsettled fills |
| `cross_chain_legs` | `(status) WHERE status IN (pending, escrowed, executing)` | Worker phase queries |
| `cross_chain_legs` | `(timeout_at) WHERE status NOT IN (confirmed, refunded)` | Timeout detection |
| `htlc_swaps` | `(status) WHERE status NOT IN (source_unlocked, refunded, expired)` | Active swap queries |
| `htlc_swaps` | `(source_timelock) WHERE status IN (created, source_locked, dest_claimed)` | Expiry detection |
| `intents` | `(status)` | Active intent queries |
| `intents` | `(stop_price) WHERE order_type = 'stop' AND status = 'open'` | Stop order monitoring |

### Read-path indexes (API queries)

| Table | Index | Purpose |
|-------|-------|---------|
| `users` | `(email) UNIQUE` | Login lookup |
| `accounts` | `(user_id)` | User's accounts |
| `bids` | `(intent_id)` | Orderbook for an intent |
| `api_keys` | `(key_prefix) WHERE revoked = FALSE` | API key authentication |
| `wallets` | `(address) UNIQUE` | Wallet address lookup |
| `transactions` | `(tx_hash) WHERE tx_hash IS NOT NULL` | Tx confirmation lookup |

### Partial indexes

Several tables use `WHERE` clauses to index only active rows, reducing index size:

- `fills`: only `WHERE settled = FALSE` — settled fills are rarely queried
- `cross_chain_legs`: only non-terminal statuses — confirmed/refunded legs are archived
- `htlc_swaps`: only non-terminal statuses
- `failed_settlements`: only `WHERE permanently_failed = FALSE`
- `twap_intents`: only `WHERE status = 'active'`

---

## 7. Enum Types Reference

| Type | Values | Used by |
|------|--------|---------|
| `asset_type` | USDC, ETH, BTC, SOL | balances, ledger, trades, markets |
| `entry_type` | DEBIT, CREDIT | ledger_entries |
| `reference_type` | TRADE, DEPOSIT, WITHDRAWAL, FEE | ledger_entries |
| `intent_status` | open, bidding, matched, executing, completed, failed, cancelled, partiallyfilled | intents |
| `order_type` | market, limit, stop | intents |
| `trade_status` | pending, settled, failed | trades |
| `execution_status` | pending, executing, completed, failed | executions |
| `leg_status` | pending, escrowed, executing, confirmed, failed, refunded | cross_chain_legs |
| `htlc_status` | created, source_locked, dest_claimed, source_unlocked, refunded, expired, failed | htlc_swaps |
| `twap_status` | active, completed, cancelled, failed | twap_intents |
| `tx_status` | pending, submitted, confirmed, failed, dropped | transactions |
| `account_type` | spot | accounts |
