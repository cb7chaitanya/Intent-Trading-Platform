# Cross-Chain Settlement: Technical Reference

This document describes the exact behavior of the cross-chain settlement system. Every state transition, SQL query, retry constant, and failure mode is derived from code. Use this to debug production incidents, modify the worker, or add new bridge adapters.

---

## 1. End-to-End Lifecycle

A cross-chain intent (e.g., ETH on Ethereum → SOL on Solana) goes through these components in order:

```
Intent creation       → src/services/intent_service.rs :: create_intent()
Fill creation         → src/engine/execution_engine.rs  (auction winner)
Leg creation          → src/cross_chain/service.rs     :: create_settlement()
Phase 1: Lock         → src/cross_chain/worker.rs      :: lock_pending_sources()
Phase 2: Verify       → src/cross_chain/worker.rs      :: verify_escrowed()
Phase 3: Release      → src/cross_chain/worker.rs      :: release_destinations()
Phase 4: Timeout      → src/cross_chain/worker.rs      :: process_timeouts()
Phase 5: Finalize     → src/cross_chain/worker.rs      :: finalize_completed()
```

### Leg creation (`service.rs:60-126`)

`create_settlement()` inserts exactly 2 rows into `cross_chain_legs`:

- **Source leg** (`leg_index = 0`): chain = source, status = `pending`, `timeout_at = NOW() + 600s`
- **Dest leg** (`leg_index = 1`): chain = dest, status = `pending`, same `timeout_at`

Both legs share `fill_id` and `intent_id`. The `(fill_id, leg_index)` pair has a UNIQUE constraint in the DB (`migrations/038_cross_chain_intents.sql`).

### Worker loop (`worker.rs:27-62`)

The worker runs an infinite loop, polling every **5 seconds** (`POLL_INTERVAL_SECS`). Each cycle executes 5 phases sequentially:

```rust
let locked    = lock_pending_sources(&service, &bridges).await;
let verified  = verify_escrowed(&service, &bridges).await;
let released  = release_destinations(&service, &bridges).await;
let timeouts  = process_timeouts(&service).await;
let completed = finalize_completed(&service, &pool).await;
```

Each phase queries for legs in a specific status, processes up to 50 per cycle (`LIMIT 50`), and transitions them. Shutdown is via `CancellationToken`.

---

## 2. State Machine

### Source leg (leg_index = 0)

```
                   lock_funds()
    Pending ──────────────────────▶ Escrowed
       │                               │
       │                               │  verify_lock() returns
       │                               │  InTransit or Completed
       │                               │
       │                               ▼
       │                           Confirmed ───▶ (Phase 5: intent = Completed)
       │                               │
       │  timeout_at < NOW()           │  timeout_at < NOW()
       ▼                               ▼
    Refunded                        Refunded
       │                               │
       └─── cascade ──▶ dest leg also refunded
```

### Destination leg (leg_index = 1)

```
                   release_funds()
    Pending ──────────────────────▶ Executing
       │                               │
       │                               │  (confirmation tracked
       │                               │   externally or by
       │                               │   next verify cycle)
       │                               ▼
       │                           Confirmed
       │
       │  timeout_at < NOW()
       ▼
    Refunded
```

### Exact transition triggers

| From | To | Triggered by | Code location | SQL WHERE |
|------|----|-------------|---------------|-----------|
| `pending` | `escrowed` | Phase 1: `bridge.lock_funds()` succeeds | `worker.rs:96` → `service.rs:136` | `WHERE id = $1` |
| `pending` | `failed` | Phase 1: `bridge.lock_funds()` fails | `worker.rs:104` → `service.rs:166` | `WHERE id = $1` |
| `escrowed` | `confirmed` | Phase 2: `bridge.verify_lock()` returns `InTransit` or `Completed` | `worker.rs:128` → `service.rs:152` | `WHERE id = $1` |
| `escrowed` | `failed` | Phase 2: `bridge.verify_lock()` returns `Failed` | `worker.rs:134` → `service.rs:166` | `WHERE id = $1` |
| `pending` (dest) | `executing` | Phase 3: `bridge.release_funds()` succeeds | `worker.rs:170` → `service.rs:146` | `WHERE id = $1` |
| Any non-terminal | `refunded` | Phase 4: `timeout_at < NOW()` | `worker.rs:196` → `service.rs:172` | `WHERE id = $1` |
| Both `confirmed` | Intent `completed` | Phase 5: both legs confirmed | `worker.rs:233` | `WHERE id = $2 AND status != 'completed'` |

### The update_leg_status() SQL (`service.rs:334-346`)

```sql
UPDATE cross_chain_legs
SET status = $2,
    tx_hash = COALESCE($3, tx_hash),
    error = COALESCE($4, error)
WHERE id = $1
```

**Known issue**: This WHERE clause does **not** guard on previous status. Two concurrent workers can race. See "Known Issues" section at the end.

---

## 3. Wormhole Bridge Internals

### Constants (`wormhole.rs:22-60`)

```
Token Bridge addresses (mainnet):
  Ethereum:  0x3ee18B2214AFF97000D974cf647E7C347E8fa585
  Solana:    wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb
  Polygon:   0x5a58505a96D1dbf8dF91cB21B54419FC36e93fdE
  Arbitrum:  0x0b2402144Bb366A632D14B83F244D2e0e21bD39c
  Base:      0x8d2de8d2f73F1F4cAB472AC9A881C9b123C79627

Chain IDs:
  solana=1, ethereum=2, polygon=5, arbitrum=23, base=30

Guardian quorum:         13 of 19
VAA poll max retries:    30
VAA poll initial delay:  2,000 ms
VAA poll max delay:      30,000 ms
Dest submit max retries: 5
Dest submit initial:     1,000 ms
```

### VAA binary format (`wormhole.rs:677-722`)

```
Byte offset    Length    Field
─────────────────────────────────────────────
0              1B        version (must be 1)
1              4B        guardian_set_index (big-endian u32)
5              1B        num_signatures (u8)
6              N×66B     signatures:
                           [0]    guardian_index (u8, 0-18)
                           [1-65] secp256k1 signature (r,s,v)

body_offset = 6 + num_signatures × 66

body_offset+0   4B       timestamp (big-endian u32)
body_offset+4   4B       nonce (big-endian u32)
body_offset+8   2B       emitter_chain (big-endian u16)
body_offset+10  32B      emitter_address (raw bytes, hex-encoded by parser)
body_offset+42  8B       sequence (big-endian u64)
body_offset+50  1B       consistency_level
body_offset+51  ...      payload (remaining bytes)
```

Minimum valid VAA size: 57 bytes (0 signatures + body header).

### Verification (`wormhole.rs:724-758`)

Off-chain verification checks:
1. `num_signatures >= 13` — rejects under-signed VAAs before submitting
2. Each guardian index must be `< 19` — no out-of-range
3. No duplicate guardian indices — checks `seen[19]` array

Full ECDSA verification happens on-chain in the destination chain's core bridge contract during `completeTransfer`. Our off-chain check is a fast sanity gate.

### lock_funds() flow (`wormhole.rs:773-853`)

```
1. Resolve chain IDs and Token Bridge address
2. Encode transferTokens(token, amount, destChainId, recipient, 0, nonce)
   Selector: 0x01930955
   Calldata: 4 + 6×32 = 196 bytes
3. Submit via eth_sendTransaction to source chain Token Bridge
4. Wait for receipt (20 attempts, backoff 2s → 15s cap)
5. Check receipt status == 0x1 (not reverted)
6. Parse LogMessagePublished event from receipt logs:
   - Match topic[0] == 0x6eb224fb001ed210e379b335e35efe88672a8ce935d981a6896b27ffdf52a3b2
   - Extract emitter from topic[1]
   - Extract sequence from first 32 bytes of log data (u64 at bytes 24-32)
7. Return LockReceipt { tx_hash, message_id = "chainId/emitter/sequence" }
```

### verify_lock() flow (`wormhole.rs:856-887`)

```
1. Call fetch_vaa_by_tx(tx_hash) — single attempt, circuit breaker protected
2. If VAA found:
   a. Call verify_vaa() (quorum + index checks)
   b. Return BridgeStatus::InTransit { message_id }
3. If not found: return BridgeStatus::Pending
4. If error: return BridgeStatus::Pending (don't fail on transient scan errors)
```

### release_funds() flow (`wormhole.rs:889-930`)

```
1. Parse message_id → (chain_id, emitter, sequence)
2. Call fetch_vaa(chain_id, emitter, sequence) — up to 30 retries with backoff
3. Call verify_vaa()
4. Call submit_vaa_to_destination(dest_chain, vaa)
5. Return dest_tx_hash
```

### fetch_vaa() retry strategy (`wormhole.rs:535-628`)

```
for attempt in 0..30:
    GET {guardian_rpc}/v1/signed_vaa/{chain_id}/{emitter}/{sequence}
    
    Circuit breaker protects the HTTP call:
      - Config: wormhole_guardian (threshold=3, reset=60s)
      - On Open: wait remaining_secs, continue
      - On Inner error: backoff, continue
    
    Response handling:
      200 + data.vaaBytes present → parse + return Vaa
      200 + data.vaaBytes null    → "pending" (guardians still signing)
      404                         → "not found" (not indexed yet)
      other                       → log, backoff
    
    Backoff: initial_ms × 2^min(attempt, 6) + 25% jitter, capped at 30s
    Sequence: 2s, 4s, 8s, 16s, 30s, 30s, 30s...
```

Total worst-case wait: ~10 minutes before giving up.

### submit_vaa_to_destination() retry strategy (`wormhole.rs:414-530`)

```
for attempt in 0..5:
    1. Dry run via eth_call (check for revert)
       - If "already completed" → return error (idempotent on-chain)
       - If other error → backoff, retry
    
    2. Submit via eth_sendTransaction
       - If tx_hash returned (0x, 66+ chars) → success
       - If error:
         - Retriable (nonce, underpriced, pool, timeout, connection) → retry
         - Fatal (other) → return error immediately
    
    Backoff: 1s × 2^min(attempt, 4) → 1s, 2s, 4s, 8s, 16s
```

### Circuit breaker configuration

| Breaker | Threshold | Reset timeout | Scope |
|---------|-----------|---------------|-------|
| `wormhole_guardian` | 3 failures | 60s | Guardian RPC |
| `wormhole_{chain}_rpc` | 5 failures | 30s | Per-chain EVM RPC |

---

## 4. Sequence Diagrams

### Happy path: Ethereum → Solana

```
Worker          Service           Wormhole Bridge       Guardian RPC      Dest Chain
  │                │                    │                    │               │
  │ ─── Phase 1: Lock ───              │                    │               │
  │                │                    │                    │               │
  │ find_pending   │                    │                    │               │
  │ source_legs()  │                    │                    │               │
  │───────────────▶│                    │                    │               │
  │ [leg: pending] │                    │                    │               │
  │◀───────────────│                    │                    │               │
  │                │                    │                    │               │
  │ bridge.lock_funds(params)           │                    │               │
  │────────────────────────────────────▶│                    │               │
  │                │                    │ eth_sendTransaction│               │
  │                │                    │──────────────────▶ (source chain)  │
  │                │                    │ tx_hash            │               │
  │                │                    │◀─────────────────── │               │
  │                │                    │ wait_for_receipt    │               │
  │                │                    │ parse logs → seq    │               │
  │ LockReceipt{tx,msg_id}             │                    │               │
  │◀────────────────────────────────────│                    │               │
  │                │                    │                    │               │
  │ execute_source_leg(leg_id, tx_hash) │                    │               │
  │───────────────▶│                    │                    │               │
  │                │ UPDATE status='escrowed', tx_hash=$1    │               │
  │                │                    │                    │               │
  │                │                    │                    │               │
  │ ─── Phase 2: Verify (next cycle) ──│                    │               │
  │                │                    │                    │               │
  │ find_escrowed  │                    │                    │               │
  │ source_legs()  │                    │                    │               │
  │───────────────▶│                    │                    │               │
  │ [leg: escrowed]│                    │                    │               │
  │◀───────────────│                    │                    │               │
  │                │                    │                    │               │
  │ bridge.verify_lock(tx_hash)         │                    │               │
  │────────────────────────────────────▶│                    │               │
  │                │                    │ GET /v1/signed_vaa_by_tx/{tx}     │
  │                │                    │───────────────────▶│               │
  │                │                    │ {data:{vaaBytes}}  │               │
  │                │                    │◀───────────────────│               │
  │                │                    │ parse_vaa()        │               │
  │                │                    │ verify_vaa() ≥13   │               │
  │ InTransit{msg_id}                  │                    │               │
  │◀────────────────────────────────────│                    │               │
  │                │                    │                    │               │
  │ confirm_leg(source_leg_id)          │                    │               │
  │───────────────▶│                    │                    │               │
  │                │ UPDATE status='confirmed', confirmed_at=NOW()          │
  │                │                    │                    │               │
  │                │                    │                    │               │
  │ ─── Phase 3: Release (next cycle) ─│                    │               │
  │                │                    │                    │               │
  │ find_ready     │                    │                    │               │
  │ dest_legs()    │   JOIN: src.status IN (escrowed,confirmed)             │
  │───────────────▶│   AND dest.status = 'pending'          │               │
  │ [dest_leg]     │                    │                    │               │
  │◀───────────────│                    │                    │               │
  │                │                    │                    │               │
  │ bridge.release_funds(params, msg_id)│                    │               │
  │────────────────────────────────────▶│                    │               │
  │                │                    │ fetch_vaa()        │               │
  │                │                    │ (30 retries)       │               │
  │                │                    │───────────────────▶│               │
  │                │                    │ VAA                │               │
  │                │                    │◀───────────────────│               │
  │                │                    │ verify_vaa()       │               │
  │                │                    │                    │               │
  │                │                    │ submit_vaa_to_destination()        │
  │                │                    │ eth_call (dry run) │               │
  │                │                    │──────────────────────────────────▶│
  │                │                    │ ok                 │              │
  │                │                    │◀──────────────────────────────────│
  │                │                    │ eth_sendTransaction│               │
  │                │                    │──────────────────────────────────▶│
  │                │                    │ dest_tx_hash       │              │
  │                │                    │◀──────────────────────────────────│
  │ dest_tx_hash   │                    │                    │               │
  │◀────────────────────────────────────│                    │               │
  │                │                    │                    │               │
  │ mark_executing(dest_leg_id, dest_tx)│                    │               │
  │───────────────▶│                    │                    │               │
  │                │ UPDATE status='executing', tx_hash=$1   │               │
  │                │                    │                    │               │
  │                │                    │                    │               │
  │ ─── Phase 5: Finalize (once dest confirmed) ──          │               │
  │                │                    │                    │               │
  │ SELECT fill_id, intent_id           │                    │               │
  │ WHERE src.status='confirmed'        │                    │               │
  │   AND dest.status='confirmed'       │                    │               │
  │   AND intent.status != 'completed'  │                    │               │
  │                │                    │                    │               │
  │ UPDATE intents SET status='completed' WHERE id=$1       │               │
  │                │                    │                    │               │
  │ ─── DONE ───   │                    │                    │               │
```

### Failure path: Timeout refund

```
Worker          Service           Bridge              Source Chain
  │                │                 │                      │
  │ Phase 1: lock_funds() succeeds   │                      │
  │ source leg → escrowed            │                      │
  │                │                 │                      │
  │ ... 10 minutes pass ...          │                      │
  │ (VAA never arrives or            │                      │
  │  dest submission fails)          │                      │
  │                │                 │                      │
  │ ─── Phase 4: Timeout ───         │                      │
  │                │                 │                      │
  │ find_timed_out_legs()            │                      │
  │ WHERE timeout_at < NOW()         │                      │
  │   AND status NOT IN              │                      │
  │     ('confirmed','refunded')     │                      │
  │───────────────▶│                 │                      │
  │ [source_leg: escrowed, expired]  │                      │
  │◀───────────────│                 │                      │
  │                │                 │                      │
  │ refund_leg(source_leg_id)        │                      │
  │───────────────▶│                 │                      │
  │                │ UPDATE status='refunded'               │
  │                │   error='Timeout refund'               │
  │                │                 │                      │
  │ ─── Cascade ── │                 │                      │
  │ (source was escrowed,            │                      │
  │  leg_index == 0)                 │                      │
  │                │                 │                      │
  │ get_settlement(fill_id)          │                      │
  │───────────────▶│                 │                      │
  │ [dest_leg: pending]              │                      │
  │◀───────────────│                 │                      │
  │                │                 │                      │
  │ refund_leg(dest_leg_id)          │                      │
  │───────────────▶│                 │                      │
  │                │ UPDATE status='refunded'               │
  │                │                 │                      │
  │ ─── Intent stays 'executing' ─── │                      │
  │ (not marked completed or failed  │                      │
  │  by Phase 5 — both legs refunded │                      │
  │  neither confirmed)              │                      │
```

### Failure path: Destination reverts

```
Worker          Service           Bridge              Dest Chain
  │                │                 │                    │
  │ Phase 3: release_funds()         │                    │
  │────────────────────────────────▶│                    │
  │                │                 │ submit_vaa()       │
  │                │                 │ eth_call (dry run) │
  │                │                 │───────────────────▶│
  │                │                 │ REVERT             │
  │                │                 │◀───────────────────│
  │                │                 │                    │
  │                │                 │ retry 5 times...   │
  │                │                 │ all revert         │
  │                │                 │                    │
  │ Err(ReleaseFailed)               │                    │
  │◀────────────────────────────────│                    │
  │                │                 │                    │
  │ fail_leg(dest_leg_id, error)     │                    │
  │───────────────▶│                 │                    │
  │                │ UPDATE status='failed', error=$1     │
  │                │                 │                    │
  │ ─── Source leg remains confirmed ──                  │
  │ ─── Phase 5 will NOT finalize ──                     │
  │   (dest is 'failed', not 'confirmed')                │
  │                │                 │                    │
  │ ─── If timeout_at passes: ───    │                    │
  │   source leg may also be refunded│                    │
  │   by Phase 4                     │                    │
```

---

## 5. Failure Scenarios (Production Debugging)

### Scenario A: Worker crashes after bridge.lock_funds() returns but before DB update

**What happens**: Funds are locked on the source chain. The DB still shows the source leg as `pending`. When the worker restarts, Phase 1 picks up the leg again and calls `lock_funds()` a second time.

**Impact**: Double lock on the source chain. The second lock consumes additional tokens from the sender.

**How to detect**: Check for two source-chain transactions with the same sender/recipient/amount within the same timeout window.

**Recovery**: The timeout refund (Phase 4) will eventually refund both on-chain locks — but only if the Token Bridge contract refunds on timeout. If it doesn't, manual intervention is needed.

**Root cause**: `lock_funds()` is not idempotent at the on-chain level. There's no check for existing on-chain lock before re-locking.

### Scenario B: Source leg confirmed, destination release fails permanently

**What happens**: Source leg reaches `confirmed`. Phase 3 calls `release_funds()` which fetches VAA and submits to dest chain. If all 5 retry attempts fail with a non-retriable error, the dest leg is marked `failed`.

**Impact**: User's funds are locked on the source chain with no destination delivery.

**How to detect**: Query `cross_chain_legs WHERE leg_index = 0 AND status = 'confirmed' AND EXISTS (SELECT 1 FROM cross_chain_legs l2 WHERE l2.fill_id = fill_id AND l2.leg_index = 1 AND l2.status = 'failed')`.

**Recovery**: Phase 4 (timeout) will refund both legs once `timeout_at` passes. The source leg transitions `confirmed → refunded`. Manual VAA resubmission is also possible using the `message_id` from the source leg.

### Scenario C: Cascading refund partially fails

**What happens**: Phase 4 refunds the source leg (line 196), then tries to refund the dest leg (line 205). If the second `refund_leg()` call fails (DB error, network issue), the source is refunded but the dest leg remains in `pending` or `executing`.

**Impact**: Inconsistent state. Source refunded, dest orphaned.

**How to detect**: The invariant checker (`src/chaos/verify.rs :: check_cross_chain_leg_consistency()`) catches this — it queries for fills where one leg is refunded but the counterpart is still active.

**Recovery**: Next Phase 4 cycle will pick up the dest leg independently (it'll also be past `timeout_at`) and refund it.

### Scenario D: VAA received but with insufficient signatures

**What happens**: `verify_vaa()` checks `num_signatures >= 13`. If the guardian RPC returns a VAA with fewer than 13 signatures (possible during guardian set rotation), the verification fails with `BridgeError::VerificationFailed`.

**Impact**: Source leg stays `escrowed`. Phase 2 will retry on next cycle. If the guardian set stabilizes, the VAA will eventually have enough signatures.

**How to detect**: Log message `wormhole_vaa_fetch_error` with error containing "Insufficient signatures".

**Recovery**: Automatic — next poll cycle retries. If it persists for 10 minutes, timeout refund kicks in.

### Scenario E: "already completed" on destination

**What happens**: `submit_vaa_to_destination()` does a dry-run `eth_call` before submitting. If the response contains "already completed", it means the VAA was already redeemed (e.g., by a previous attempt that succeeded but whose response was lost).

**Impact**: `release_funds()` returns `Err(ReleaseFailed("VAA already redeemed"))`. The dest leg is marked `failed` by the worker.

**How to detect**: Log message `lz_message_failed` or `dest_submit_fatal` with "already redeemed".

**Recovery**: This is actually a success case (funds were delivered). Manual DB correction: update dest leg to `confirmed` with the actual dest tx hash from the chain explorer.

---

## 6. Idempotency and Safety

### DB-level protections

| Protection | Mechanism | Location |
|-----------|-----------|----------|
| Unique legs per fill | `UNIQUE (fill_id, leg_index)` | `migrations/038_cross_chain_intents.sql` |
| Finalization guard | `WHERE status != 'completed'` in UPDATE | `worker.rs:233` |
| `rows_affected() == 0` check | Returns `LegNotFound` if no row matched | `service.rs:348` |

### On-chain protections

| Protection | Mechanism | Chain |
|-----------|-----------|-------|
| Duplicate VAA redemption | Token Bridge rejects already-completed VAAs | EVM (Wormhole) |
| Tx nonce ordering | EVM nonce prevents duplicate sends | All EVM chains |

### What is NOT protected

| Gap | Description | Risk |
|-----|-------------|------|
| No previous-status guard | `update_leg_status()` uses `WHERE id = $1` without `AND status = $expected` | Two workers can race and overwrite status |
| No on-chain lock check | `lock_funds()` doesn't verify whether source chain already has a lock | Double-lock on crash recovery |
| Non-atomic cascade | Source and dest refunds are two separate SQL calls | Partial refund state on crash |
| Finalization race | Phase 5 reads legs then updates intent without re-checking | Stale read → premature finalization |

---

## 7. Bridge Registry and Route Selection

`BridgeRegistry` (`bridge_registry.rs`) holds an ordered `Vec<Arc<dyn BridgeAdapter>>`. Route selection uses **first match** — the first registered bridge that returns `true` from `supports_route(source, dest)` wins.

Registration order in `main.rs`:
1. Wormhole (registered first)
2. LayerZero (registered second)

This means:
- **Ethereum ↔ Solana**: Wormhole wins (LayerZero doesn't support Solana)
- **Ethereum ↔ Arbitrum**: Wormhole wins (registered first, both support it)
- If Wormhole is removed, LayerZero would handle EVM ↔ EVM routes

### Supported routes

| Bridge | ethereum | solana | polygon | arbitrum | base |
|--------|----------|--------|---------|----------|------|
| Wormhole | Yes | Yes | Yes | Yes | Yes |
| LayerZero | Yes | No | Yes | Yes | Yes |

---

## 8. Metrics and Observability

### Counters (Prometheus)

| Metric | Labels | Incremented when |
|--------|--------|-----------------|
| `cross_chain_legs_processed` | `status=locked` | Phase 1 succeeds |
| `cross_chain_legs_processed` | `status=executing` | Phase 3 succeeds |
| `cross_chain_legs_processed` | `status=refunded` | Phase 4 refunds a leg |
| `cross_chain_legs_processed` | `status=confirmed` | Phase 5 finalizes intent |
| `cross_chain_timeouts_total` | — | Phase 4 refunds a timed-out leg |

### Gauges

| Metric | Updated | Meaning |
|--------|---------|---------|
| `cross_chain_pending_legs` | End of each cycle | Count of timed-out + ready-for-release legs |

### Key log events

| Event | Level | When |
|-------|-------|------|
| `cross_chain_settlement_created` | INFO | Legs inserted |
| `bridge_locked` | INFO | Phase 1 success |
| `bridge_lock_failed` | ERROR | Phase 1 failure |
| `source_confirmed` | INFO | Phase 2 confirms source |
| `bridge_released` | INFO | Phase 3 success |
| `bridge_release_failed` | ERROR | Phase 3 failure |
| `timeout` | WARN | Phase 4 refunds |
| `cross_chain_completed` | INFO | Phase 5 finalizes |
| `cross_chain_cycle` | INFO | End of worker cycle (only if work done) |

---

## 9. Known Issues and Recommended Fixes

### Issue 1: No previous-status guard on transitions

**Location**: `service.rs:334-346`
**Risk**: Two concurrent workers can set conflicting statuses.
**Fix**: Change `WHERE id = $1` to `WHERE id = $1 AND status = $expected_previous_status`. Return `InvalidState` if `rows_affected() == 0`.

### Issue 2: Non-atomic cascading refund

**Location**: `worker.rs:196-209`
**Risk**: Crash between source refund and dest refund.
**Fix**: Wrap both `refund_leg()` calls in a single SQL transaction.

### Issue 3: Finalization race

**Location**: `worker.rs:218-244`
**Risk**: Legs could change state between the SELECT and the UPDATE.
**Fix**: Use a CTE or subquery in the UPDATE: `UPDATE intents SET status = 'completed' WHERE id = $1 AND EXISTS (both legs confirmed)`.

### Issue 4: Double lock on crash recovery

**Location**: `worker.rs:94`
**Risk**: If worker crashes after `lock_funds()` succeeds but before DB update, re-processing locks funds twice.
**Fix**: Before calling `lock_funds()`, check if source chain already has a transaction from this sender to this Token Bridge for this amount (via `eth_getTransactionCount` or similar).

### Issue 5: "Already completed" misclassified as failure

**Location**: `wormhole.rs:443-446`
**Risk**: Returns `Err(ReleaseFailed)`, worker marks leg as `failed`. But funds were actually delivered.
**Fix**: Detect "already completed" and mark leg as `confirmed` instead of `failed`. Look up the dest tx hash on-chain.
