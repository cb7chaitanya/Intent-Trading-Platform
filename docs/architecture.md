# Architecture

## System Overview

IntentX is structured as a monolithic Rust backend with multiple background workers, a Next.js frontend, and on-chain settlement programs on EVM and Solana.

```
                         ┌──────────────────────┐
                         │       Nginx           │
                         │  (TLS, rate limiting) │
                         └──────┬───────┬────────┘
                                │       │
                    ┌───────────▼─┐   ┌─▼──────────────┐
                    │  Frontend   │   │  API Gateway    │
                    │  (Next.js)  │   │  (Auth proxy)   │
                    └─────────────┘   └────────┬────────┘
                                               │
                              ┌────────────────▼────────────────┐
                              │      Intent Trading Platform     │
                              │           (Axum, :3000)          │
                              │                                  │
                              │  ┌──────────┐  ┌──────────────┐ │
                              │  │  API      │  │ WebSocket    │ │
                              │  │ Handlers  │  │ Feed Server  │ │
                              │  └────┬─────┘  └──────┬───────┘ │
                              │       │               │         │
                              │  ┌────▼───────────────▼──────┐  │
                              │  │     Intent Service         │  │
                              │  │  (create, cancel, amend)   │  │
                              │  └────────────┬───────────────┘  │
                              │               │                  │
                              │  ┌────────────▼───────────────┐  │
                              │  │     Auction Engine          │  │
                              │  │  (bid collection, ranking)  │  │
                              │  └────────────┬───────────────┘  │
                              │               │                  │
                              │  ┌────────────▼───────────────┐  │
                              │  │    Execution Engine         │  │
                              │  │  (match → fill → settle)   │  │
                              │  └────────────┬───────────────┘  │
                              │               │                  │
                              │  ┌────────────▼───────────────┐  │
                              │  │    Settlement Engine        │  │
                              │  │  (atomic balance transfer)  │  │
                              │  └────────────────────────────┘  │
                              │                                  │
                              │  Background Workers:             │
                              │  ├── Settlement retry worker     │
                              │  ├── Cross-chain settlement      │
                              │  ├── HTLC swap worker            │
                              │  ├── TWAP scheduler              │
                              │  ├── TWAP completion listener    │
                              │  ├── Intent expiry worker        │
                              │  ├── Stop order monitor          │
                              │  ├── Partition archival          │
                              │  └── Tx confirmation worker      │
                              └──────────────┬──────────────────┘
                                             │
                        ┌────────────────────┼────────────────────┐
                        │                    │                    │
                   ┌────▼────┐         ┌─────▼────┐        ┌─────▼─────┐
                   │PostgreSQL│         │  Redis   │        │ Chains    │
                   │  (data)  │         │ (cache)  │        │(EVM, Sol) │
                   └──────────┘         └──────────┘        └───────────┘
```

## Intent Lifecycle

### Single-Chain Intent

```
    User                  Platform                Solver              Chain
     │                       │                      │                   │
     │  POST /intents        │                      │                   │
     │──────────────────────▶│                      │                   │
     │                       │                      │                   │
     │                       │  Lock balance         │                   │
     │                       │  (available → locked) │                   │
     │                       │                      │                   │
     │                       │  Publish to WS        │                   │
     │                       │─────────────────────▶│                   │
     │                       │                      │                   │
     │                       │  Auction (10s)        │                   │
     │                       │◀─────────────────────│                   │
     │                       │  POST /bids (×N)     │                   │
     │                       │                      │                   │
     │                       │  Select best bid      │                   │
     │                       │  Create fill          │                   │
     │                       │                      │                   │
     │                       │  settle_fill()        │                   │
     │                       │  (atomic tx):         │                   │
     │                       │  1. Unlock buyer      │                   │
     │                       │  2. Debit buyer       │                   │
     │                       │  3. Credit seller     │                   │
     │                       │  4. Credit buyer      │                   │
     │                       │  5. Platform fee      │                   │
     │                       │  6. Solver fee        │                   │
     │                       │  7. Ledger entries    │                   │
     │                       │  8. Mark settled      │                   │
     │                       │                      │                   │
     │  Balance updated      │                      │                   │
     │◀──────────────────────│                      │                   │
```

### Cross-Chain Intent (Wormhole Path)

```
    User          Platform        Wormhole Bridge      Guardians      Dest Chain
     │                │                  │                  │              │
     │ Create intent  │                  │                  │              │
     │ (ETH→SOL)     │                  │                  │              │
     │───────────────▶│                  │                  │              │
     │                │                  │                  │              │
     │                │  Create legs:    │                  │              │
     │                │  source(eth,pending)                │              │
     │                │  dest(sol,pending) │                │              │
     │                │                  │                  │              │
     │                │ ── Phase 1: Lock ──                 │              │
     │                │  transferTokens  │                  │              │
     │                │─────────────────▶│                  │              │
     │                │  tx_hash         │                  │              │
     │                │◀─────────────────│                  │              │
     │                │  source→escrowed │                  │              │
     │                │                  │                  │              │
     │                │ ── Phase 2: Verify ──               │              │
     │                │  fetch VAA       │                  │              │
     │                │─────────────────▶│                  │              │
     │                │                  │  Sign VAA        │              │
     │                │                  │  (13/19 quorum)  │              │
     │                │  signed VAA      │◀─────────────────│              │
     │                │◀─────────────────│                  │              │
     │                │  source→confirmed│                  │              │
     │                │                  │                  │              │
     │                │ ── Phase 3: Release ──              │              │
     │                │  completeTransfer│                  │              │
     │                │─────────────────────────────────────────────────▶│
     │                │                  │                  │   dest_tx   │
     │                │◀─────────────────────────────────────────────────│
     │                │  dest→confirmed  │                  │              │
     │                │                  │                  │              │
     │                │ ── Phase 5: Finalize ──             │              │
     │                │  intent→completed│                  │              │
     │                │                  │                  │              │
     │ Funds received │                  │                  │              │
     │◀───────────────│                  │                  │              │
```

**Timeout path**: If the VAA isn't signed within 10 minutes, Phase 4 triggers. Both legs are refunded and the user's source chain funds are returned.

### Cross-Chain Leg State Machine

```
   Pending ──────▶ Escrowed ──────▶ Confirmed
      │                │                │
      │                │                └──▶ (both confirmed → intent Completed)
      │                │
      └──▶ Failed      └──▶ Refunded (timeout)
```

## HTLC Atomic Swap Lifecycle

HTLCs provide cryptographic guarantees for cross-chain swaps without trusting the bridge. The secret-hash binding ensures either both parties get paid or neither does.

```
    Platform          Source Chain       Solver        Dest Chain
       │                   │               │              │
       │ generate secret S │               │              │
       │ compute H=SHA256(S)               │              │
       │                   │               │              │
       │ ── Step 1: Lock source ──         │              │
       │ lock(H, timelock=30min)           │              │
       │──────────────────▶│               │              │
       │   source_lock_tx  │               │              │
       │◀──────────────────│               │              │
       │   Created → SourceLocked          │              │
       │                   │               │              │
       │ ── Step 2: Solver mirrors ──      │              │
       │                   │  lock(H, T/2) │              │
       │                   │──────────────▶│              │
       │                   │               │  dest_lock   │
       │                   │               │─────────────▶│
       │                   │               │              │
       │ ── Step 3: Claim dest (reveal S) ──              │
       │ claim(secret=S)   │               │              │
       │──────────────────────────────────────────────────▶│
       │   dest_claim_tx   │               │   tokens     │
       │◀──────────────────────────────────────────────────│
       │   SourceLocked → DestClaimed      │              │
       │                   │               │              │
       │ ── Step 4: Solver unlocks source (S is public) ──│
       │                   │  claim(S)     │              │
       │                   │◀──────────────│              │
       │                   │   tokens      │              │
       │                   │──────────────▶│              │
       │   DestClaimed → SourceUnlocked    │              │
       │                   │               │              │
       │ ── Timeout path (if no claim before T) ──        │
       │                   │               │              │
       │ refund()          │               │              │
       │──────────────────▶│               │              │
       │   tokens returned │               │              │
       │◀──────────────────│               │              │
       │   → Refunded                      │              │
```

**HTLC State Machine:**
```
   Created ──▶ SourceLocked ──▶ DestClaimed ──▶ SourceUnlocked (terminal, success)
      │              │
      │              └──▶ Refunded (terminal, timeout)
      │
      └──▶ Failed (terminal, error)
```

## Auction System

The auction engine runs as part of the intent lifecycle:

1. **Intent created** → status = `Open`
2. **Bidding starts** → status = `Bidding`, solvers notified via WebSocket
3. **10-second auction window** → solvers submit `POST /bids`
4. **Best bid selected** → highest `amount_out` wins, status = `Matched`
5. **Fill created** → links intent to winning solver
6. **Execution** → status = `Executing`, settlement begins
7. **Settlement completes** → status = `Completed`

Solver bids include:
- `amount_out`: how much the user receives (higher is better)
- `fee`: solver's fee for execution

## Settlement Engine

The settlement engine is the core of fund safety. `settle_fill()` executes inside a PostgreSQL transaction:

```sql
BEGIN;
  SELECT * FROM fills WHERE id = $1 FOR UPDATE;        -- Lock fill row
  -- If already settled, return AlreadySettled (idempotent)
  UPDATE balances SET locked_balance -= amount ...;     -- Unlock buyer
  UPDATE balances SET available_balance += amount ...;  -- Credit seller
  UPDATE balances SET available_balance += amount ...;  -- Credit buyer
  UPDATE balances SET available_balance -= amount ...;  -- Debit seller
  UPDATE balances SET available_balance += fee ...;     -- Platform fee
  UPDATE balances SET available_balance += fee ...;     -- Solver fee
  INSERT INTO ledger_entries ...;                       -- 4-6 entries
  UPDATE fills SET settled = TRUE, settled_at = NOW();
COMMIT;
```

This is atomic — if any step fails, the entire transaction rolls back.

## Background Workers

| Worker | Poll interval | What it does |
|--------|-------------|-------------|
| Settlement retry | 5s | Retries failed settlements (max 5 attempts, exponential backoff) |
| Cross-chain settlement | 5s | 5-phase cycle: lock → verify → release → timeout → finalize |
| HTLC swap | 5s | 5-phase cycle: lock → monitor → claim → unlock → refund |
| TWAP scheduler | 5s | Submits scheduled TWAP child intents |
| TWAP listener | Stream | Records child intent completions |
| Intent expiry | 30s | Expires intents past their deadline |
| Stop order monitor | 5s | Triggers stop orders when oracle price crosses threshold |
| Partition archival | 1h | Archives old partition data |
| Tx confirmation | 5s | Polls chain for transaction confirmations |

## Database Schema (Key Tables)

```
users ─────────┐
               │
accounts ──────┤──▶ balances (per asset)
               │         │
               │    ledger_entries (double-entry)
               │
intents ───────┤──▶ fills ──▶ executions
               │         │
               │    cross_chain_legs (source + dest)
               │         │
               │    htlc_swaps (atomic swap state)
               │
markets ───────┤──▶ market_trades
               │         │
               │    market_prices (oracle)
               │
solvers ───────┤──▶ bids
               │
twap_intents ──┤──▶ twap_child_intents
```

39 migrations build this schema. See [Database Schema](database-schema.md) for the complete ERD.

## Security Layers

| Layer | Mechanism |
|-------|-----------|
| Transport | TLS 1.2/1.3 via Nginx, HSTS |
| Authentication | JWT with HMAC-SHA256, key rotation |
| Authorization | RBAC with granular permissions |
| CSRF | Double-submit token (Redis-backed) |
| Rate limiting | Sliding window (Redis), per-user + per-endpoint |
| API keys | SHA-256 hashed, prefix-indexed |
| Wallet encryption | AES-256-GCM with master key |
| On-chain | ReentrancyGuard, Pausable, PDA validation |
| Resilience | Circuit breakers, exponential backoff, chaos testing |
| Monitoring | 51 Prometheus alert rules, Grafana dashboards |
