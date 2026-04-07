# Mainnet Deployment Checklist

Every item is a pass/fail gate. Do not deploy if any Critical or High item fails. Each item includes the exact command or query to verify it.

---

## 1. Security Audit Remediation

### Critical (MUST fix before mainnet)

- [ ] **C-1**: Solidity `settle()` requires EIP-712 signature from buyer and seller
  ```bash
  # Verify: grep for signature verification in settle function
  grep -n "ecrecover\|ECDSA\|EIP712" contracts/src/IntentXSettlement.sol
  # Must return matches
  ```

- [ ] **C-2**: Solidity `updateAuthority()` uses two-step transfer with timelock
  ```bash
  grep -n "pendingAuthority\|acceptAuthority\|timelock" contracts/src/IntentXSettlement.sol
  ```

- [ ] **C-3**: Solana Settlement validates buyer/seller account ownership
  ```bash
  grep -n "buyer.*constraint\|seller.*constraint\|BuyerMismatch" programs/intentx-settlement/src/lib.rs
  ```

- [ ] **C-4**: Solana Settlement has fill_id deduplication
  ```bash
  grep -n "fill_record\|FillRecord\|b\"fill\"" programs/intentx-settlement/src/lib.rs
  ```

- [ ] **C-5**: Backend unsigned tx data has HMAC authentication
  ```bash
  grep -n "hmac\|HMAC\|verify_hmac" src/wallet/ethereum.rs
  ```

- [ ] **C-6**: Ethereum signing validates chain_id against allowlist
  ```bash
  grep -n "chain_id.*allowlist\|ALLOWED_CHAIN" src/wallet/eth_sign.rs
  ```

- [ ] **C-8**: JWT enforces HS256 algorithm explicitly
  ```bash
  grep -n "Algorithm::HS256\|Algorithm::HS" src/auth/jwt.rs
  # Must NOT find: Header::default() without algorithm specification
  ```

### High (MUST fix before mainnet)

- [ ] **H-1**: Solidity admin functions respect pause
- [ ] **H-6**: CSRF requires both header AND cookie tokens
  ```bash
  grep -n "cookie_token.*is_none\|Missing CSRF cookie" src/csrf/middleware.rs
  ```
- [ ] **H-7**: API key comparison uses constant-time equality
  ```bash
  grep -n "ct_eq\|constant_time\|subtle" src/api_keys/service.rs
  ```
- [ ] **H-8**: JWT never falls back to static secret
  ```bash
  grep -n "NoSigningKey\|Err.*no.*key" src/auth/jwt.rs
  ```

---

## 2. Smart Contract Deployment

### Solidity (EVM)

- [ ] **Contract compiled with optimizer**
  ```bash
  cd contracts && forge build --optimizer-runs 200
  # Verify: solc 0.8.24, 200 runs (matches foundry.toml)
  ```

- [ ] **Deployed to target chain**
  ```bash
  # Record the deployed address:
  export SETTLEMENT_CONTRACT=0x...
  ```

- [ ] **Constructor parameters verified**
  ```bash
  # Verify on-chain:
  cast call $SETTLEMENT_CONTRACT "authority()" --rpc-url $RPC
  cast call $SETTLEMENT_CONTRACT "feeRecipient()" --rpc-url $RPC
  cast call $SETTLEMENT_CONTRACT "feeBps()" --rpc-url $RPC
  # feeBps must be <= 5000 (MAX_FEE_BPS)
  ```

- [ ] **Contract verified on block explorer**
  ```bash
  forge verify-contract $SETTLEMENT_CONTRACT IntentXSettlement --chain $CHAIN_ID
  ```

- [ ] **Duplicate fillId protection confirmed**
  ```bash
  # Attempt to settle same fillId twice:
  # Second call must revert with DuplicateFillId
  ```

### Solana (Anchor)

- [ ] **Programs deployed to mainnet-beta**
  ```bash
  CLUSTER=mainnet-beta ./scripts/anchor_deploy.sh
  # Record program IDs:
  export SETTLEMENT_PROGRAM=...
  export HTLC_PROGRAM=...
  ```

- [ ] **Program IDs match frontend config**
  ```bash
  grep "SETTLEMENT_PROGRAM_ID\|HTLC_PROGRAM_ID" frontend/lib/solana-config.ts
  # Must match deployed program IDs
  ```

- [ ] **Settlement program initialized**
  ```bash
  # Verify config account exists and has correct authority
  solana account $CONFIG_PDA --url mainnet-beta
  ```

- [ ] **Authority keypair is NOT the deployer keypair**
  ```bash
  # Authority must be a separate, more-secured key
  # Deployer key can be rotated out after deployment
  ```

---

## 3. Configuration Validation

### Secrets (no defaults allowed in production)

- [ ] **JWT_SECRET is not the default**
  ```bash
  # Must NOT be "change-me-in-production"
  echo $JWT_SECRET | wc -c
  # Must be >= 32 characters
  ```

- [ ] **WALLET_MASTER_KEY is not all zeros**
  ```bash
  # Must NOT be 00000000...
  [ "$WALLET_MASTER_KEY" != "0000000000000000000000000000000000000000000000000000000000000000" ] && echo "OK" || echo "FAIL: using default key"
  ```

- [ ] **INTERNAL_SIGNING_SECRET is not the default**
  ```bash
  [ "$INTERNAL_SIGNING_SECRET" != "change-me-internal-signing-secret" ] && echo "OK" || echo "FAIL"
  ```

### Chain configuration

- [ ] **CHAIN_ID matches target network**
  ```bash
  # Ethereum mainnet = 1, NOT 11155111 (Sepolia)
  echo "CHAIN_ID=$CHAIN_ID"
  # Verify against RPC:
  curl -s -X POST $ETH_RPC_URL -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' | jq -r '.result'
  ```

- [ ] **RPC_ENDPOINT points to mainnet (not devnet/testnet)**
  ```bash
  # Must NOT contain: devnet, testnet, sepolia, goerli
  echo $RPC_ENDPOINT | grep -vi "devnet\|testnet\|sepolia\|goerli" && echo "OK" || echo "FAIL"
  echo $SOLANA_RPC_ENDPOINT | grep -vi "devnet\|testnet" && echo "OK" || echo "FAIL"
  ```

- [ ] **Cross-chain RPC URLs are mainnet**
  ```bash
  for var in ETH_RPC_URL POLYGON_RPC_URL ARBITRUM_RPC_URL BASE_RPC_URL; do
    echo "$var=${!var}" | grep -vi "testnet\|sepolia\|goerli\|mumbai" && echo "OK" || echo "FAIL: $var"
  done
  ```

- [ ] **Token Bridge addresses are mainnet (not testnet)**
  ```bash
  # Wormhole Token Bridge Ethereum mainnet: 0x3ee18B2214AFF97000D974cf647E7C347E8fa585
  grep "0x3ee18B2214AFF97000D974cf647E7C347E8fa585" src/cross_chain/wormhole.rs && echo "OK"
  ```

### Fee and risk parameters

- [ ] **fee_rate is reasonable**
  ```bash
  grep "fee_rate" config.toml
  # Default: 0.001 (0.1%). Typical production: 0.001 - 0.01
  ```

- [ ] **rate_limit_per_minute is production-appropriate**
  ```bash
  grep "rate_limit_per_minute" config.toml
  # Default: 120. Production: 60-300 depending on expected traffic
  ```

- [ ] **max_price_deviation is reasonable**
  ```bash
  grep "max_price_deviation" config.toml
  # Default: 0.20 (20%). Production: 0.05-0.20
  ```

- [ ] **daily_volume_limit is set**
  ```bash
  grep "daily_volume_limit" config.toml
  # Default: 10,000,000. Set based on expected daily volume
  ```

### Circuit breaker thresholds

- [ ] **Circuit breakers are configured (not using dev defaults)**
  ```
  Verify these values in config or code are appropriate for production:
  
  ethereum_rpc:      threshold=5,  reset=30s   (trips after 5 consecutive failures)
  solana_rpc:        threshold=5,  reset=20s
  wormhole_guardian: threshold=3,  reset=60s
  layerzero_api:     threshold=3,  reset=60s
  price_oracle:      threshold=3,  reset=15s
  ```

### Database

- [ ] **pg_max_connections is production-sized**
  ```bash
  grep "pg_max_connections" config.toml
  # Default: 5. Production: 20-50 depending on worker count
  ```

- [ ] **ENVIRONMENT is set to "production"**
  ```bash
  echo "ENVIRONMENT=$ENVIRONMENT"
  # Must be "production", NOT "dev" or "docker"
  ```

---

## 4. Infrastructure

### Service health

- [ ] **All services are running**
  ```bash
  docker compose ps --format "table {{.Name}}\t{{.State}}\t{{.Health}}"
  # All must show "running" and healthy services must show "healthy"
  ```

- [ ] **Platform health check passes**
  ```bash
  curl -sf http://localhost:3000/health/ready | jq
  # Must return: {"status":"ok","services":{"db":"ok","redis":"ok","engine":"ok"}}
  ```

- [ ] **Database migrations are applied**
  ```bash
  docker compose exec postgres psql -U postgres -d intent_trading \
    -c "SELECT count(*) as tables FROM pg_tables WHERE schemaname='public';"
  # Must show 15+ tables
  ```

- [ ] **Redis is responding**
  ```bash
  docker compose exec redis redis-cli ping
  # Must return: PONG
  ```

### Database backup

- [ ] **Backup runs successfully**
  ```bash
  docker compose exec pg-backup /scripts/backup.sh
  echo $?  # Must be 0
  ```

- [ ] **Backup can be restored to a test database**
  ```bash
  # Restore to a separate test DB and verify table count
  docker compose exec postgres createdb -U postgres intent_trading_restore_test
  gunzip -c /backups/latest.sql.gz | docker compose exec -T postgres psql -U postgres -d intent_trading_restore_test
  docker compose exec postgres psql -U postgres -d intent_trading_restore_test \
    -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';"
  docker compose exec postgres dropdb -U postgres intent_trading_restore_test
  ```

- [ ] **Backup schedule is active**
  ```bash
  # pg-backup container must be running (production profile)
  docker compose --profile production ps pg-backup
  ```

### Monitoring

- [ ] **Prometheus targets are UP**
  ```bash
  curl -s http://localhost:9090/api/v1/targets | jq '.data.activeTargets[] | {job: .labels.job, health: .health}'
  # All must show "health": "up"
  ```

- [ ] **Grafana dashboard loads**
  ```bash
  curl -sf -u admin:$GRAFANA_PASSWORD http://localhost:3002/api/dashboards/uid/intentx-trading | jq '.dashboard.title'
  # Must return: "IntentX Trading Platform"
  ```

- [ ] **Alert rules are loaded**
  ```bash
  curl -s http://localhost:9090/api/v1/rules | jq '.data.groups | length'
  # Must return: 12 (alert groups)
  ```

- [ ] **Loki is receiving logs**
  ```bash
  curl -s "http://localhost:3100/loki/api/v1/query?query={service=\"intent-trading\"}&limit=1" | jq '.data.result | length'
  # Must return: >= 1
  ```

### TLS

- [ ] **HTTPS is working**
  ```bash
  curl -sf https://$DOMAIN/health/ready
  # Must not fail with certificate errors
  ```

- [ ] **HTTP redirects to HTTPS**
  ```bash
  curl -sf -o /dev/null -w "%{redirect_url}" http://$DOMAIN/
  # Must start with https://
  ```

- [ ] **HSTS header is set**
  ```bash
  curl -sI https://$DOMAIN/ | grep -i strict-transport
  # Must show: Strict-Transport-Security: max-age=63072000
  ```

---

## 5. Security

### Key management

- [ ] **No plaintext private keys in environment**
  ```bash
  # Check for raw hex keys in env
  env | grep -i "private\|secret\|master_key" | grep -v "=change-me" | while read line; do
    value=$(echo "$line" | cut -d= -f2)
    if [ ${#value} -eq 64 ] && echo "$value" | grep -qP '^[0-9a-fA-F]+$'; then
      echo "WARNING: possible raw key in env: $(echo "$line" | cut -d= -f1)"
    fi
  done
  ```

- [ ] **Authority keys are in a hardware wallet or KMS**
  ```
  Document the key management setup:
  - Solidity authority key: [ ] Hardware wallet / [ ] KMS / [ ] Hot wallet (NOT acceptable)
  - Solana authority key:   [ ] Hardware wallet / [ ] KMS / [ ] Hot wallet (NOT acceptable)
  - Wallet master key:      [ ] KMS / [ ] Env var (acceptable if server is secured)
  ```

- [ ] **JWT secret is from a secure random source**
  ```bash
  # Generate if not already set:
  openssl rand -hex 32
  ```

### Authentication

- [ ] **Registration is rate-limited**
  ```bash
  # Nginx auth rate limit: 5 req/s with burst 3
  grep "limit_req_zone.*auth" infra/nginx/nginx.conf
  ```

- [ ] **API key hash comparison is constant-time**
  ```bash
  grep -n "ct_eq\|ConstantTime" src/api_keys/service.rs
  ```

- [ ] **CSRF token is validated on all POST/PUT/DELETE**
  ```bash
  # Test: submit POST without X-CSRF-Token
  curl -s -o /dev/null -w "%{http_code}" -X POST http://localhost:3000/intents \
    -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{}'
  # Must return: 403
  ```

---

## 6. Dry Run (Full Flow Test)

Execute this on the production deployment BEFORE announcing to users. Use small amounts.

### Step 1: Register + deposit

```bash
# Register
RESP=$(curl -s -X POST https://$DOMAIN/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"dryrun@test.com","password":"dryrun123456"}')
TOKEN=$(echo $RESP | jq -r '.token')
USER_ID=$(echo $RESP | jq -r '.user_id')

# Create account
ACCT=$(curl -s -X POST https://$DOMAIN/api/accounts \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"user_id\":\"$USER_ID\"}")
ACCOUNT_ID=$(echo $ACCT | jq -r '.id')

# Get CSRF token
CSRF=$(curl -s https://$DOMAIN/api/csrf-token | jq -r '.token')

# Deposit
curl -s -X POST https://$DOMAIN/api/balances/deposit \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-CSRF-Token: $CSRF" \
  -H "Content-Type: application/json" \
  -d "{\"account_id\":\"$ACCOUNT_ID\",\"asset\":\"USDC\",\"amount\":1000}"
```

- [ ] Registration returns 201 with token
- [ ] Account creation returns 201
- [ ] Deposit returns 200 with updated balance

### Step 2: Submit intent

```bash
DEADLINE=$(($(date +%s) + 3600))
INTENT=$(curl -s -X POST https://$DOMAIN/api/intents \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-CSRF-Token: $CSRF" \
  -H "Content-Type: application/json" \
  -d "{
    \"user_id\":\"$USER_ID\",
    \"account_id\":\"$ACCOUNT_ID\",
    \"token_in\":\"USDC\",
    \"token_out\":\"ETH\",
    \"amount_in\":100,
    \"min_amount_out\":1,
    \"deadline\":$DEADLINE
  }")
INTENT_ID=$(echo $INTENT | jq -r '.id')
echo "Intent: $INTENT_ID Status: $(echo $INTENT | jq -r '.status')"
```

- [ ] Intent returns 201 with status "Open"
- [ ] Balance shows locked_balance increased

### Step 3: Verify auction fires

```bash
# Wait 15 seconds for auction to complete
sleep 15
curl -s https://$DOMAIN/api/intents/$INTENT_ID \
  -H "Authorization: Bearer $TOKEN" | jq '.status'
```

- [ ] Status progresses (Open → Bidding → Matched or Expired)
- [ ] If solver bot is running: status reaches Matched or Completed

### Step 4: Check settlement

```bash
# Check balances after settlement
curl -s https://$DOMAIN/api/balances/$ACCOUNT_ID \
  -H "Authorization: Bearer $TOKEN" | jq
```

- [ ] If settled: USDC decreased, ETH increased (or received asset)
- [ ] Ledger entries exist for the trade

### Step 5: Withdraw

```bash
curl -s -X POST https://$DOMAIN/api/balances/withdraw \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-CSRF-Token: $CSRF" \
  -H "Content-Type: application/json" \
  -d "{\"account_id\":\"$ACCOUNT_ID\",\"asset\":\"USDC\",\"amount\":100}"
```

- [ ] Withdrawal returns 200
- [ ] Balance decreased by withdrawn amount

### Step 6: Verify invariants

```bash
# Run invariant checker against production DB
docker compose exec postgres psql -U postgres -d intent_trading -c "
  -- No negative balances
  SELECT count(*) as negative_balances FROM balances
  WHERE available_balance < 0 OR locked_balance < 0;

  -- Balance conservation
  SELECT b.asset, b.total as balance_sum, COALESCE(l.net, 0) as ledger_net,
         b.total - COALESCE(l.net, 0) as diff
  FROM (SELECT asset, SUM(available_balance + locked_balance) as total FROM balances GROUP BY asset) b
  LEFT JOIN (SELECT asset, SUM(CASE WHEN entry_type='CREDIT' THEN amount ELSE -amount END) as net FROM ledger_entries GROUP BY asset) l
  ON b.asset = l.asset
  WHERE b.total != COALESCE(l.net, 0);
"
```

- [ ] Zero negative balances
- [ ] Zero balance/ledger mismatches

---

## 7. Post-Deployment Monitoring

### First 24 hours: watch these metrics

| Metric | Alert threshold | Dashboard panel | Action if exceeded |
|--------|----------------|-----------------|-------------------|
| API p99 latency | > 2.0s | "API Latency (p50/p95)" | Check DB connections, add indexes |
| 5xx error rate | > 5% | "API Error Rates" | Check logs: `{service="intent-trading"} \|= "ERROR"` |
| Settlement failure rate | > 10% | "Settlement Success Rate" | Check `failed_settlements` table, review retry queue |
| Settlement p99 latency | > 5.0s | "Settlement Failures" | Check chain RPC latency, circuit breaker state |
| DB connections | > 80% of max | Not on dashboard | Increase `pg_max_connections`, check for leaks |
| Redis memory | > 85% | Not on dashboard | Check CSRF token accumulation, tune TTLs |
| Disk space | < 10% remaining | Not on dashboard | Expand volume, run partition archival |
| Backup freshness | > 25 hours | Not on dashboard | Check pg-backup container logs |
| WebSocket connections | Drop > 50% in 1h | "WebSocket Connections" | Check nginx config, intent-trading memory |
| Oracle price staleness | > 120s | Not on dashboard | Check oracle source, price feed connectivity |
| Cross-chain pending legs | Growing over time | Not on dashboard | Check bridge RPC connectivity, guardian availability |

### Queries for incident response

```sql
-- Active settlements stuck in non-terminal state
SELECT id, status, created_at, timeout_at
FROM cross_chain_legs
WHERE status NOT IN ('confirmed', 'refunded', 'failed')
  AND created_at < NOW() - INTERVAL '30 minutes'
ORDER BY created_at;

-- HTLC swaps past timelock but not resolved
SELECT id, status, source_timelock, source_chain, dest_chain
FROM htlc_swaps
WHERE source_timelock < NOW()
  AND status NOT IN ('source_unlocked', 'refunded', 'expired', 'failed');

-- Failed settlements awaiting retry
SELECT id, fill_id, retry_count, last_error, next_retry_at
FROM failed_settlements
WHERE permanently_failed = FALSE
ORDER BY next_retry_at;

-- Orphan locked balances
SELECT b.account_id, b.asset, b.locked_balance
FROM balances b
WHERE b.locked_balance > 0
  AND NOT EXISTS (
    SELECT 1 FROM intents i JOIN accounts a ON a.user_id::text = i.user_id
    WHERE a.id = b.account_id AND i.status IN ('open','bidding','matched','executing')
  );
```

### Rollback procedure

If critical issues are found post-deployment:

1. **Pause all contracts**
   ```bash
   # Solidity
   cast send $SETTLEMENT_CONTRACT "pause()" --rpc-url $RPC --private-key $AUTHORITY_KEY
   ```

2. **Stop workers** (settlements, cross-chain, HTLC)
   ```bash
   docker compose stop intent-trading solver-bot
   ```

3. **Do NOT delete data** — the DB contains the source of truth for all in-flight settlements

4. **Investigate** — check logs, DB state, on-chain state

5. **Fix and redeploy** — apply fix, run dry run again, then resume
   ```bash
   # Solidity
   cast send $SETTLEMENT_CONTRACT "unpause()" --rpc-url $RPC --private-key $AUTHORITY_KEY
   docker compose up -d intent-trading solver-bot
   ```
