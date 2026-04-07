# Configuration Reference

All settings are defined in `src/config/settings.rs`. Every parameter has a hardcoded default. Override via:

1. `config.toml` in the working directory
2. Environment variables (bare: `DATABASE_URL`, or prefixed: `ITX__DATABASE_URL`)
3. `.env` file (loaded at startup)

Priority: env vars > config.toml > hardcoded defaults.

---

## Trading Engine

### `auction_duration_secs`

| | |
|---|---|
| **What it controls** | How long the auction window stays open for solver bids after an intent enters `Bidding` status. |
| **Default** | `10` |
| **Type** | `u64` (seconds) |
| **Used by** | `AuctionEngine::new()` in `main.rs:240` |

**How it works**: When an intent transitions to `Bidding`, a timer starts. Solvers submit bids via `POST /bids`. After `auction_duration_secs`, the engine selects the best bid (highest `amount_out`) and creates a fill.

**Dev**: `10` is fine. Short auctions mean faster testing.

**Prod**: `10` is typical. Increase to `15-30` if solvers are on high-latency connections. Decrease to `5` for high-frequency markets where speed matters more than solver participation.

**Risk if too low**: Solvers don't have time to calculate and submit bids. Intents expire with zero bids (alert: `NoBidsPerAuction`).

**Risk if too high**: Users wait longer for settlement. Funds are locked during the entire auction window.

---

### `execution_duration_secs`

| | |
|---|---|
| **What it controls** | Time allowed for the execution engine to process a matched intent and create the on-chain settlement transaction. |
| **Default** | `3` |
| **Type** | `u64` (seconds) |
| **Used by** | `ExecutionEngine::new()` in `main.rs:245` |

**Dev**: `3` is fine.

**Prod**: `3-5`. This is the gap between auction completion and settlement submission. If chain RPCs are slow, increase to `5-10`.

**Risk if too low**: Execution times out before the settlement tx is submitted. Intent marked as failed; user's funds stay locked until manual intervention or expiry worker runs.

---

### `fee_rate`

| | |
|---|---|
| **What it controls** | Platform trading fee as a decimal fraction. Applied to every settlement. |
| **Default** | `0.001` (0.1%) |
| **Type** | `f64` |
| **Used by** | `settle_fill()` in `settlement/engine.rs`, `settle_intent_fills()` |

**Calculation**: `platform_fee = amount * fee_rate`. The fee is deducted from the seller's received amount and credited to the platform account (`00000000-0000-0000-0000-000000000001`).

**Dev**: `0.001` is fine.

**Prod**: `0.001` (0.1%) to `0.003` (0.3%) is typical for trading platforms. Must be coordinated with the on-chain `feeBps` parameter in the Solidity/Anchor contracts.

**Risk if too high**: Users avoid the platform. Solvers demand higher spreads to compensate.

**Risk if set to 0**: Platform earns no revenue. The settlement engine still functions correctly.

**Risk if > 1.0**: Fee exceeds the trade amount. Settlement will fail with insufficient balance or produce negative outputs. **Never set above 0.5.**

---

### `max_price_deviation`

| | |
|---|---|
| **What it controls** | Maximum allowed deviation between a bid's implied price and the oracle price. Prevents manipulation. |
| **Default** | `0.20` (20%) |
| **Type** | `f64` |
| **Used by** | `RiskEngine::validate_bid_price()` in `risk/service.rs:262`, cross-market arbitrage detection at line 329 |

**How it works**: When a solver submits a bid, the risk engine compares the bid's implied price to the oracle price. If `|bid_price - oracle_price| / oracle_price > max_price_deviation`, the bid is rejected with `RiskRejected`.

**Dev**: `0.20` (20%) is permissive enough for test environments with volatile mock prices.

**Prod**: `0.05` (5%) for stablecoin pairs. `0.10-0.20` for volatile pairs (ETH/BTC). Consider per-market configuration if you add it.

**Risk if too tight**: Legitimate bids rejected during volatile markets. Auctions get zero bids.

**Risk if too loose**: Malicious solvers submit extreme prices. Users get terrible fills. A solver could bid $1 for $1000 of ETH and win if no other solver participates.

---

### `rate_limit_per_minute`

| | |
|---|---|
| **What it controls** | Maximum API requests per user per sliding window. Enforced via Redis. |
| **Default** | `120` |
| **Type** | `u64` |
| **Used by** | `gateway/rate_limit.rs` |

**Dev**: `120` is fine.

**Prod**: `60-120` for authenticated users. Nginx also enforces per-IP limits: `60 req/s` for API, `5 req/s` for auth endpoints (configured in `infra/nginx/nginx.conf`).

**Risk if too low**: Legitimate users and solver bots get rate-limited during normal operation. Solver bots need to submit bids quickly during auctions.

**Risk if too high**: Susceptible to API abuse, credential stuffing, or enumeration attacks.

---

### `rate_limit_window_secs`

| | |
|---|---|
| **What it controls** | The sliding window duration for rate limiting. |
| **Default** | `60` |
| **Type** | `u64` (seconds) |

**Dev/Prod**: `60` is standard. No reason to change this.

---

### `max_intents_per_minute`

| | |
|---|---|
| **What it controls** | Maximum number of intents a single user can submit per minute. Prevents spam. |
| **Default** | `30` |
| **Type** | `u64` |
| **Used by** | `RiskEngine::check_rate_limit()` in `risk/service.rs:415` |

**Dev**: `30` is fine.

**Prod**: `10-30`. Lower for retail users. Higher if institutional users submit many small orders.

**Risk if too low**: Power users and TWAP orders get blocked. TWAP creates child intents every `interval_secs`, so a 60-slice TWAP at 1-second intervals needs 60 intents/minute.

**Risk if too high**: A single user can flood the auction engine and consume worker resources.

---

### `daily_volume_limit`

| | |
|---|---|
| **What it controls** | Maximum total volume (in smallest token unit) a single user can trade per day. Resets at midnight UTC. |
| **Default** | `10,000,000` |
| **Type** | `u64` |
| **Used by** | `RiskEngine::check_volume_limit()` in `risk/service.rs:423` |

**Dev**: `10,000,000` is fine.

**Prod**: Set based on your risk appetite. For a USDC-denominated platform with 6 decimals, `10,000,000` = $10. For an ETH platform with 18 decimals, `10,000,000` = 0.00000000001 ETH. **Adjust based on your token's decimal precision.**

**Risk if too low**: Users can't trade meaningful amounts.

**Risk if too high**: A compromised account can drain large amounts before detection.

---

## Infrastructure

### `database_url`

| | |
|---|---|
| **Default** | `postgres://postgres:postgres@127.0.0.1:5432/intent_trading` |

**Prod**: Use a dedicated PostgreSQL instance with SSL. Format: `postgres://user:password@host:5432/intent_trading?sslmode=require`

**Risk if using default in prod**: Anyone on the network can connect with `postgres:postgres`. **Critical security issue.**

---

### `redis_url`

| | |
|---|---|
| **Default** | `redis://127.0.0.1:6379` |

**Prod**: Use a dedicated Redis instance. Consider `redis://user:password@host:6379` with TLS if available.

**What depends on Redis**: Rate limiting, CSRF tokens, event bus (intent created → solver notification), cache, nonce tracking for request replay protection.

---

### `pg_max_connections`

| | |
|---|---|
| **What it controls** | PostgreSQL connection pool size per service instance. |
| **Default** | `5` |
| **Type** | `u32` |

**Dev**: `5` is fine.

**Prod**: `20-50`. Each background worker holds a connection during its poll cycle. With 9 workers polling every 5s, you need at least 10 connections. Add headroom for API handlers. Must be less than PostgreSQL's `max_connections` (default 100) divided by the number of service instances.

**Risk if too low**: Connection pool exhaustion under load. API requests queue up waiting for a connection. Alert: `DbConnectionsNearLimit`.

**Risk if too high**: Exceeds PostgreSQL's `max_connections`. New connections are rejected. All services fail simultaneously.

---

### `partition_retention_months`

| | |
|---|---|
| **What it controls** | How many months of historical data to keep in partitioned tables (trades, fills, ledger_entries, executions, market_trades). |
| **Default** | `6` |
| **Type** | `i32` |

**Dev**: `6` is fine.

**Prod**: `6-12`. Data older than this is dropped by the partition archival worker (runs hourly). Ensure backups cover the retention window if historical data is needed.

**Risk if too low**: Reporting queries fail. Historical trade data is lost.

**Risk if too high**: Database grows unbounded. Disk fills up. Alert: `DiskSpaceLow`.

---

## Security

### `jwt_secret`

| | |
|---|---|
| **Default** | `change-me-in-production` |

**Prod**: Generate with `openssl rand -hex 32`. Must be at least 32 characters. Shared between intent-trading and api-gateway services.

**Risk if using default**: Anyone can forge JWT tokens and impersonate any user. **Critical.**

---

### `wallet_master_key`

| | |
|---|---|
| **Default** | `0000...0000` (64 hex zeros) |
| **Type** | 32-byte hex string |

Used for AES-256-GCM encryption of wallet private keys in the `wallets` table.

**Prod**: Generate with `openssl rand -hex 32`. Store in a KMS or secrets manager. If this key is lost, all encrypted wallet keys become unrecoverable.

**Risk if using default**: All wallet private keys are encrypted with a known key. Anyone with DB access can decrypt them. **Critical.**

---

### `internal_signing_secret`

| | |
|---|---|
| **Default** | `change-me-internal-signing-secret` |

HMAC secret for inter-service request signing (gateway → platform). Prevents request forgery between services.

**Prod**: Generate with `openssl rand -hex 32`. Only the gateway and platform services need this value.

---

## Blockchain

### `rpc_endpoint`

| | |
|---|---|
| **Default** | `http://127.0.0.1:8545` |

Primary EVM RPC endpoint for the settlement chain adapter.

**Prod**: Use a reliable RPC provider (Alchemy, Infura, QuickNode) or self-hosted node. The circuit breaker trips after 5 consecutive failures with a 30s reset.

---

### `chain_id`

| | |
|---|---|
| **Default** | `1` (Ethereum mainnet) |
| **Type** | `u64` |

Must match the chain the `rpc_endpoint` connects to. Used in EIP-155 transaction signing. A mismatch means signed transactions are invalid on the target chain.

| Network | chain_id |
|---------|----------|
| Ethereum mainnet | `1` |
| Sepolia testnet | `11155111` |
| Polygon | `137` |
| Arbitrum One | `42161` |
| Base | `8453` |

---

### `solana_rpc_endpoint`

| | |
|---|---|
| **Default** | `https://api.devnet.solana.com` |

**Prod**: Use `https://api.mainnet-beta.solana.com` or a dedicated RPC provider (Helius, Triton). The default points to devnet — transactions will succeed but on the wrong network.

---

### Cross-chain RPC URLs (environment variables only)

These are not in `config.toml`. Set via environment:

| Variable | Default | Used by |
|----------|---------|---------|
| `ETH_RPC_URL` | `https://eth.llamarpc.com` | Wormhole + LayerZero bridge |
| `POLYGON_RPC_URL` | `https://polygon.llamarpc.com` | Wormhole + LayerZero bridge |
| `ARBITRUM_RPC_URL` | `https://arbitrum.llamarpc.com` | Wormhole + LayerZero bridge |
| `BASE_RPC_URL` | `https://base.llamarpc.com` | Wormhole + LayerZero bridge |

**Prod**: Use dedicated RPC providers. Public endpoints (llamarpc) have rate limits and may be unreliable under load.

---

## Circuit Breakers

Circuit breakers are hardcoded in `src/circuit_breaker.rs`. They are not configurable via config.toml — changing them requires a code change and redeploy.

| Breaker | Failure threshold | Reset timeout | What it protects |
|---------|-------------------|---------------|-----------------|
| `ethereum_rpc` | 5 failures | 30s | EVM RPC calls for settlement |
| `solana_rpc` | 5 failures | 20s | Solana RPC calls |
| `wormhole_guardian` | 3 failures | 60s | Wormhole guardian VAA RPC |
| `layerzero_api` | 3 failures | 60s | LayerZero Scan API |
| `price_oracle` | 3 failures | 15s | Oracle price feed sources |
| `wormhole_{chain}_rpc` | 5 failures | 30s | Per-chain Wormhole bridge RPC |
| `layerzero_{chain}_rpc` | 5 failures | 30s | Per-chain LayerZero bridge RPC |

**State machine**: `Closed` → (threshold failures) → `Open` → (reset timeout) → `HalfOpen` → (probe succeeds) → `Closed` / (probe fails) → `Open`

**In Open state**: All calls are rejected immediately with `CircuitError::Open`. No external call is made. This prevents cascading failures when an upstream service is down.

**In HalfOpen state**: One probe call is allowed through. If it succeeds, the breaker closes. If it fails, the breaker reopens.

**Risk if threshold too low**: Transient errors (single timeout, one 500) trip the breaker unnecessarily. All calls blocked for the reset timeout duration.

**Risk if threshold too high**: A failing service receives too many calls before the breaker trips. Timeouts accumulate, increasing latency for all users.

**Risk if reset timeout too short**: Breaker reopens before the upstream service recovers. Constant flip-flopping between Open and HalfOpen.

**Risk if reset timeout too long**: Service recovery is delayed. Even after the upstream is healthy, the breaker stays open for the full timeout.

---

## Logging and Observability

### `log_level`

| | |
|---|---|
| **Default** | `info` |
| **Options** | `debug`, `info`, `warn`, `error` |

**Dev**: `debug` for full visibility. Warning: very verbose.

**Prod**: `info`. Switch to `debug` temporarily during incident investigation.

### `environment`

| | |
|---|---|
| **Default** | `dev` |
| **Options** | `dev`, `docker`, `production` |

Controls behavior differences like internal signing enforcement (only in `production` and `docker` modes).

### Environment-only variables

| Variable | Default | Description |
|----------|---------|-------------|
| `LOG_FORMAT` | (unset = text) | Set to `json` for structured logging (recommended for prod with Loki) |
| `OTEL_ENABLED` | (unset = false) | Set to `true` to enable OpenTelemetry tracing to Jaeger |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | — | Jaeger collector endpoint, e.g. `http://jaeger:4317` |
| `OTEL_SERVICE_NAME` | — | Service name in traces, e.g. `intentx-trading` |
| `CHAOS_ENABLED` | (unset = false) | Set to `true` to enable fault injection. **Never in production.** |

---

## Dev vs Prod Quick Reference

| Parameter | Dev | Prod | Why |
|-----------|-----|------|-----|
| `jwt_secret` | `change-me-in-production` | `openssl rand -hex 32` | Default is public knowledge |
| `wallet_master_key` | `000...000` | `openssl rand -hex 32` | Encrypts wallet keys |
| `internal_signing_secret` | `change-me-...` | `openssl rand -hex 32` | Inter-service auth |
| `environment` | `dev` | `production` | Enables signing enforcement |
| `pg_max_connections` | `5` | `20-50` | Workers need connections |
| `log_level` | `debug` | `info` | Noise reduction |
| `LOG_FORMAT` | (unset) | `json` | Loki parsing |
| `chain_id` | `11155111` (Sepolia) | `1` (mainnet) | Wrong chain = lost funds |
| `solana_rpc_endpoint` | devnet | mainnet-beta | Wrong network = test tokens |
| `rpc_endpoint` | `localhost:8545` | Provider URL | Local node vs production |
| `CHAOS_ENABLED` | `true` (optional) | **never** | Injects real failures |
| `fee_rate` | `0.001` | `0.001-0.003` | Revenue model |
| `max_price_deviation` | `0.20` | `0.05-0.20` | Per-market risk |
| `daily_volume_limit` | `10000000` | Based on decimals | Adjust per token precision |
