# Incident Response Runbook

This document is for 3am incidents. No theory. Every section follows the same format: what fired, what it means, what to do right now, how to dig deeper, when to escalate.

**Dashboards**: Grafana at `http://localhost:3002` (admin/admin)
**Logs**: Loki via Grafana Explore, or `docker compose logs <service>`
**Metrics**: Prometheus at `http://localhost:9090`
**Traces**: Jaeger at `http://localhost:16686`

---

## 1. Settlement Failures

### Alert: `SettlementFailuresHigh`
**Fires when**: > 3 settlement failures in 15 minutes
**Severity**: Critical

#### What it means
The settlement engine is failing to execute atomic balance transfers. Users' funds are locked (deducted from `available_balance`, held in `locked_balance`) but not delivered to the counterparty.

#### Likely causes
1. DB transaction deadlock (two settlements competing for the same balance row)
2. Insufficient seller balance (seller withdrew between match and settlement)
3. Solver account not found (solver_id doesn't map to a valid account)
4. Fill already settled (duplicate event processing — benign)

#### Immediate actions

```bash
# 1. Check how many failures are pending
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT count(*), permanently_failed FROM failed_settlements GROUP BY permanently_failed;"

# 2. See the actual errors
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT id, fill_id, retry_count, last_error, next_retry_at
   FROM failed_settlements WHERE permanently_failed = FALSE
   ORDER BY next_retry_at DESC LIMIT 10;"

# 3. Check for DB locks
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT pid, state, query, wait_event_type, wait_event
   FROM pg_stat_activity
   WHERE state != 'idle' AND query NOT LIKE '%pg_stat%'
   ORDER BY query_start;"

# 4. Check settlement worker logs
docker compose logs intent-trading 2>&1 | grep -i "settle\|settlement" | tail -30
```

#### If cause is deadlock
```bash
# Kill stuck queries
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
   WHERE state = 'active' AND query_start < NOW() - INTERVAL '30 seconds'
   AND query LIKE '%balances%FOR UPDATE%';"
```

#### If cause is insufficient balance
```bash
# Check the specific fill's buyer and seller balances
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT f.id as fill_id, f.intent_id, i.user_id, i.token_in, i.amount_in,
          b.available_balance, b.locked_balance
   FROM fills f
   JOIN intents i ON i.id = f.intent_id
   JOIN accounts a ON a.user_id::text = i.user_id
   JOIN balances b ON b.account_id = a.id AND b.asset = i.token_in::asset_type
   WHERE f.id = '<FILL_ID>';"
```

#### Escalation
- If `permanently_failed` count > 10: page the on-call engineer
- If deadlocks persist after killing queries: investigate `pg_max_connections` and pool sizing
- If all failures are `InsufficientBalance`: check if a bug is allowing intents without proper balance locks

---

### Alert: `SettlementRetryQueueCritical`
**Fires when**: > 200 entries in `failed_settlements` table
**Severity**: Critical

#### What it means
Settlements are failing faster than the retry worker can process them. Backlog is growing.

#### Immediate actions

```bash
# Check retry worker health
docker compose logs intent-trading 2>&1 | grep "settlement_retry\|retry_worker" | tail -20

# Check the error distribution
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT left(last_error, 80) as error, count(*)
   FROM failed_settlements WHERE permanently_failed = FALSE
   GROUP BY left(last_error, 80) ORDER BY count DESC LIMIT 10;"

# If all errors are the same: fix the root cause, then reset retry timestamps
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "UPDATE failed_settlements SET next_retry_at = NOW(), retry_count = 0
   WHERE permanently_failed = FALSE;"
```

#### Escalation
- If queue > 500 and growing: pause new intent creation until backlog clears
- If all failures are a single error class: this is likely a systemic issue (RPC down, DB schema drift)

---

## 2. Cross-Chain Timeouts

### Alert: `cross_chain_timeouts_total` increasing
**No Prometheus alert defined** — monitor via Grafana or manual query.

#### What it means
Cross-chain settlements are not completing within the 10-minute timeout window (`DEFAULT_TIMEOUT_SECS = 600`). Funds locked on the source chain are being refunded.

#### Likely causes
1. Guardian network slow to sign VAAs (Wormhole)
2. Destination chain RPC unreachable (circuit breaker tripped)
3. Bridge adapter's `release_funds()` failing (dest tx reverts)
4. Worker crashed mid-flight (restart picks up, but timeout may have passed)

#### Immediate actions

```bash
# 1. Find stuck cross-chain legs
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT id, intent_id, chain, status, tx_hash, error, timeout_at,
          EXTRACT(EPOCH FROM (timeout_at - NOW()))::int as secs_remaining
   FROM cross_chain_legs
   WHERE status NOT IN ('confirmed', 'refunded', 'failed')
   ORDER BY created_at ASC LIMIT 20;"

# 2. Check circuit breaker state in logs
docker compose logs intent-trading 2>&1 | grep "circuit_open\|circuit_half" | tail -10

# 3. Check if guardian RPC is reachable
curl -s https://wormhole-v2-mainnet-api.certus.one/v1/heartbeats | jq '.entries | length'
# Should return 19 (one per guardian)

# 4. Check destination chain RPC
curl -s -X POST $ETH_RPC_URL -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' | jq '.result'
```

#### If guardian RPC is down
```bash
# Switch to backup guardian endpoint
# Edit docker-compose.yml or env: WORMHOLE_GUARDIAN_RPC=https://api.wormholescan.io
docker compose restart intent-trading
```

#### If destination chain RPC is down
```bash
# Circuit breaker will auto-recover after reset timeout (30s for chain RPCs)
# Monitor: the "circuit_half_open" log means it's testing recovery
docker compose logs -f intent-trading 2>&1 | grep "circuit"
```

#### Manual VAA redemption
If a VAA was signed but never submitted to the destination:

```bash
# Find the message_id from the source leg
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT id, tx_hash, status FROM cross_chain_legs
   WHERE intent_id = '<INTENT_ID>' AND leg_index = 0;"

# Fetch the VAA manually
curl -s "https://wormhole-v2-mainnet-api.certus.one/v1/signed_vaa_by_tx/<SOURCE_TX_HASH>" | jq '.data.vaaBytes'

# If VAA exists, it can be manually submitted to the destination Token Bridge
```

#### Escalation
- If > 5 timeouts in 1 hour: page on-call, disable cross-chain intent creation
- If guardian RPC unreachable for > 30 min: contact Wormhole team (they have a Discord)

---

## 3. RPC Failures

### Alert: `PlatformDown` or `GatewayDown`
**Fires when**: `up{job="intent-trading"} == 0` for 30s
**Severity**: Critical

#### What it means
The core platform process is not responding to Prometheus scrapes. All API, settlement, and worker functionality is offline.

#### Immediate actions

```bash
# 1. Check container status
docker compose ps intent-trading
docker compose ps api-gateway

# 2. Check if it's restarting (crash loop)
docker compose logs intent-trading --tail 50 2>&1 | grep -i "panic\|fatal\|error\|failed to"

# 3. Check if dependencies are up
docker compose exec postgres pg_isready -U postgres
docker compose exec redis redis-cli ping

# 4. If container is stopped, restart it
docker compose up -d intent-trading

# 5. If crash-looping, check the startup error
docker compose logs intent-trading 2>&1 | head -30
# Common: "Failed to connect to Postgres" = DB is down
# Common: "Failed to run migrations" = schema issue
# Common: "Address already in use" = port conflict
```

#### If Postgres is the cause
```bash
docker compose logs postgres --tail 30
docker compose restart postgres
# Wait for health check to pass
sleep 10 && docker compose exec postgres pg_isready -U postgres
# Then restart the platform
docker compose restart intent-trading
```

#### Escalation
- If platform won't start after 3 restart attempts: page on-call engineer
- If the error is in migration code: do NOT manually modify the DB — rollback the deployment

---

### Chain RPC circuit breaker tripped

#### Symptoms
- Log: `circuit_open: wormhole_ethereum_rpc` or similar
- Cross-chain settlements stalling
- Settlement engine cannot submit on-chain transactions

#### Immediate actions

```bash
# 1. Check which breakers are open
docker compose logs intent-trading 2>&1 | grep "circuit_open" | tail -10

# 2. Test the RPC endpoint directly
curl -s -X POST $ETH_RPC_URL -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' | jq

# 3. If RPC is actually down, switch to backup
# Update ETH_RPC_URL in .env and restart:
docker compose restart intent-trading

# 4. Circuit breaker auto-recovers:
#    ethereum_rpc: resets after 30s, probes with one request
#    wormhole_guardian: resets after 60s
#    Monitor for "circuit_half_open" → "circuit_closed" in logs
```

---

## 4. Database Issues

### Alert: `DbConnectionsNearLimit`
**Fires when**: Connections > 80% of `max_connections`
**Severity**: Critical

#### Immediate actions

```bash
# 1. See current connection count and who's using them
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT count(*) as total,
          count(*) FILTER (WHERE state = 'active') as active,
          count(*) FILTER (WHERE state = 'idle') as idle,
          count(*) FILTER (WHERE state = 'idle in transaction') as idle_in_tx
   FROM pg_stat_activity WHERE datname = 'intent_trading';"

# 2. Find idle-in-transaction connections (likely leaks)
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT pid, state, query_start, left(query, 100) as query
   FROM pg_stat_activity
   WHERE state = 'idle in transaction'
     AND query_start < NOW() - INTERVAL '60 seconds'
   ORDER BY query_start;"

# 3. Kill idle-in-transaction connections older than 5 minutes
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
   WHERE state = 'idle in transaction'
     AND query_start < NOW() - INTERVAL '5 minutes';"

# 4. Check pg_max_connections in config
docker compose exec postgres psql -U postgres -c "SHOW max_connections;"
# Default: 100. Platform pool default: 5. If running multiple services, increase.
```

#### If connections keep growing
The platform's pool size is `pg_max_connections` in config.toml (default 5). If multiple services share the DB:
- intent-trading: 5 connections
- api-gateway: 5 connections
- solver-bot: 1 connection
- Total: ~11

If you see > 50 connections, there's likely a connection leak. Restart the platform service:
```bash
docker compose restart intent-trading api-gateway
```

### Alert: `DbQueryLatencyP99Critical`
**Fires when**: p99 query latency > 250ms
**Severity**: Critical

#### Immediate actions

```bash
# 1. Find the slow queries
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT left(query, 120) as query, calls, mean_exec_time::int as avg_ms,
          max_exec_time::int as max_ms
   FROM pg_stat_statements
   ORDER BY mean_exec_time DESC LIMIT 10;"

# 2. Check for missing indexes on hot tables
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT relname, seq_scan, idx_scan, seq_scan - idx_scan as diff
   FROM pg_stat_user_tables
   WHERE seq_scan > idx_scan AND seq_scan > 1000
   ORDER BY diff DESC LIMIT 10;"

# 3. Check for table bloat
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT relname, n_dead_tup, n_live_tup,
          CASE WHEN n_live_tup > 0 THEN round(n_dead_tup::numeric/n_live_tup, 2) END as dead_ratio
   FROM pg_stat_user_tables
   WHERE n_dead_tup > 1000
   ORDER BY n_dead_tup DESC LIMIT 10;"

# 4. If bloated, run vacuum
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "VACUUM ANALYZE intents; VACUUM ANALYZE fills; VACUUM ANALYZE balances;"
```

#### Escalation
- If a specific query is consistently > 1s: it needs an index. File a ticket.
- If all queries are slow: check disk I/O (`DiskIOHighLatency` alert), consider moving to faster storage.

---

## 5. Redis Failures

### Alert: `RedisDown`
**Fires when**: `up{job="redis"} == 0`
**Severity**: Critical

#### What breaks when Redis is down
- **Rate limiting**: All requests bypass rate limits
- **CSRF tokens**: All POST/PUT/DELETE requests fail with 403
- **Event bus**: Intent creation events not published, solvers don't see new intents
- **Cache**: All cache reads miss, fall through to DB (increased DB load)
- **Nonce tracking**: Request signature replay protection disabled

#### Immediate actions

```bash
# 1. Check Redis container
docker compose ps redis
docker compose logs redis --tail 20

# 2. Try to restart
docker compose restart redis
sleep 5
docker compose exec redis redis-cli ping  # Should return PONG

# 3. If Redis won't start, check disk space
df -h $(docker volume inspect intent-trading_redis-data -f '{{.Mountpoint}}' 2>/dev/null || echo "/var/lib/docker")

# 4. If data corruption, start fresh (cached data is ephemeral)
docker compose down redis
docker volume rm intent-trading_redis-data 2>/dev/null
docker compose up -d redis

# 5. After Redis recovers, restart the platform to re-establish connections
docker compose restart intent-trading api-gateway
```

#### Impact assessment
```bash
# Check if rate limiting is bypassed (compare request rate to normal)
curl -s http://localhost:9090/api/v1/query?query=rate(api_requests_total[1m]) | jq '.data.result[0].value[1]'
# If >> normal (e.g., 100x baseline), attackers may be exploiting the outage
```

### Alert: `RedisHighMemory`
**Fires when**: Memory > 85% of maxmemory
**Severity**: Warning

```bash
# Check what's using memory
docker compose exec redis redis-cli info memory
docker compose exec redis redis-cli dbsize

# Flush expired keys
docker compose exec redis redis-cli --scan --pattern "csrf:*" | head -5
# CSRF tokens should auto-expire (60s TTL). If accumulating, the TTL isn't working.

# If memory is critical, flush all caches (transient data, safe to clear)
docker compose exec redis redis-cli flushdb
```

---

## 6. Balance Invariant Violations

**No Prometheus alert** — run manually or after chaos tests.

#### What it means
Funds have been created or destroyed in the system. This is the most serious class of bug.

#### How to detect

```bash
# Run the full invariant checker
docker compose exec postgres psql -U postgres -d intent_trading -c "
  -- INV-1: Balance conservation
  SELECT 'balance_vs_ledger' as check, b.asset::text,
         b.total as balance_sum, COALESCE(l.net, 0) as ledger_net,
         b.total - COALESCE(l.net, 0) as discrepancy
  FROM (SELECT asset, SUM(available_balance + locked_balance) as total FROM balances GROUP BY asset) b
  LEFT JOIN (SELECT asset, SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net FROM ledger_entries GROUP BY asset) l
  ON b.asset = l.asset
  WHERE b.total != COALESCE(l.net, 0);

  -- INV-6: Negative balances
  SELECT 'negative_balance' as check, account_id, asset::text,
         available_balance, locked_balance
  FROM balances WHERE available_balance < 0 OR locked_balance < 0;

  -- INV-3: Orphan locks
  SELECT 'orphan_lock' as check, b.account_id, b.asset::text, b.locked_balance
  FROM balances b WHERE b.locked_balance > 0
  AND NOT EXISTS (
    SELECT 1 FROM intents i JOIN accounts a ON a.user_id::text = i.user_id
    WHERE a.id = b.account_id AND i.status IN ('open','bidding','matched','executing')
  );
"
```

#### If discrepancy found

```bash
# 1. IMMEDIATELY: Pause settlements to prevent further damage
# Solidity:
cast send $SETTLEMENT_CONTRACT "pause()" --rpc-url $RPC --private-key $AUTHORITY_KEY
# Platform:
docker compose stop solver-bot

# 2. Identify which transactions caused the discrepancy
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT le.id, le.account_id, le.asset::text, le.amount, le.entry_type::text,
          le.reference_type::text, le.created_at
   FROM ledger_entries le
   ORDER BY created_at DESC LIMIT 50;"

# 3. Cross-reference with balance mutations
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT id, account_id, asset::text, available_balance, locked_balance, updated_at
   FROM balances
   ORDER BY updated_at DESC LIMIT 20;"

# 4. DO NOT manually adjust balances — document the discrepancy and escalate
```

#### Escalation
**Always escalate balance invariant violations.** This indicates a bug in the settlement engine, a race condition, or a crash-recovery gap. The verification-strategy.md document lists 15 known risks — check if the violation matches one.

---

## 7. Worker Crashes

### Alert: `HighRestartRate`
**Fires when**: > 3 restarts in 1 hour
**Severity**: Warning

#### What it means
The platform process is crash-looping. Each restart recovers background workers, but in-flight operations may be lost or duplicated.

#### Workers affected by a crash

| Worker | Poll interval | What's at risk |
|--------|-------------|---------------|
| Cross-chain settlement | 5s | Source locked on-chain, DB still pending → double lock on restart |
| HTLC swap | 5s | Secret revealed on-chain, DB not updated → swap stuck |
| Settlement retry | 5s | Failed settlements not retried during downtime |
| TWAP scheduler | 5s | Child intents not submitted on time |
| Intent expiry | 30s | Expired intents not cancelled, funds stay locked |

#### Immediate actions

```bash
# 1. Find the crash cause
docker compose logs intent-trading --tail 100 2>&1 | grep -B5 "panic\|fatal\|SIGKILL\|OOM"

# 2. Check if OOM killed
dmesg | grep -i "oom\|killed" | tail -5

# 3. Check memory usage
docker stats --no-stream intent-trading

# 4. If OOM: increase container memory limit in docker-compose.yml
# deploy:
#   resources:
#     limits:
#       memory: 4G

# 5. If panic: the stack trace in logs will show the exact function
# Common panics:
#   "called unwrap() on None" → null data from DB or RPC
#   "index out of bounds"     → malformed VAA or response parsing
#   "connection refused"      → DB or Redis down during startup
```

#### Post-crash verification

After the worker restarts, verify no operations were duplicated:

```bash
# Check for duplicate cross-chain locks
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT intent_id, count(*) FROM cross_chain_legs
   WHERE leg_index = 0
   GROUP BY intent_id HAVING count(*) > 1;"

# Check for HTLC swaps stuck in source_locked with no secret
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT id, status, source_chain, dest_chain,
          EXTRACT(EPOCH FROM (source_timelock - NOW()))::int as secs_left
   FROM htlc_swaps
   WHERE status = 'source_locked' AND secret IS NULL
   ORDER BY created_at;"

# Check for intents stuck in executing with no active settlement
docker compose exec postgres psql -U postgres -d intent_trading -c \
  "SELECT i.id, i.status, i.created_at
   FROM intents i
   WHERE i.status = 'executing'
     AND NOT EXISTS (SELECT 1 FROM cross_chain_legs l WHERE l.intent_id = i.id)
     AND NOT EXISTS (SELECT 1 FROM fills f WHERE f.intent_id = i.id AND f.settled = FALSE)
   ORDER BY i.created_at;"
```

#### Escalation
- If crash cause is a panic in settlement code: page on-call, stop solver bot until fixed
- If OOM: increase memory and monitor — if it keeps growing, there's a memory leak
- If crash cause is unknown: collect core dump, escalate to engineering

---

## Quick Reference: Alert → Section

| Alert | Section | Severity |
|-------|---------|----------|
| `SettlementFailuresHigh` | 1 | Critical |
| `SettlementFailureRateHigh` | 1 | Critical |
| `SettlementLatencyP99Critical` | 1 | Critical |
| `SettlementRetryQueueCritical` | 1 | Critical |
| `PlatformDown` | 3 | Critical |
| `GatewayDown` | 3 | Critical |
| `HealthCheckFailing` | 3 | Critical |
| `DbConnectionsNearLimit` | 4 | Critical |
| `DbQueryLatencyP99Critical` | 4 | Critical |
| `RedisDown` | 5 | Critical |
| `HighRestartRate` | 7 | Warning |
| `CriticalMemoryUsage` | 7 | Critical |
| `DiskSpaceLow` | 4 | Critical |
| `BackupStale` | 4 | Critical |
| `BackupFailed` | 4 | Critical |
| `ErrorRateSpike` | 3 | Critical |
| `OraclePriceCriticallyStale` | 3 | Critical |
| `WebSocketConnectionsDrop` | 3 | Critical |
| `AuctionsStalled` | 3 | Critical |
