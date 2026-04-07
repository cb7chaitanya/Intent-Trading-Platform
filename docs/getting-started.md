# Getting Started

## Prerequisites

- **Docker** 24+ and **Docker Compose** v2
- **8 GB RAM** minimum (PostgreSQL + Redis + monitoring stack)
- **Ports available**: 80, 443, 3000, 3002, 4000, 5432, 6379, 9090, 16686

Optional (for development without Docker):
- Rust 1.86+ (`rustup install stable`)
- Node.js 20+ and npm
- PostgreSQL 16
- Redis 7

## Step 1: Clone and Configure

```bash
git clone <repo-url>
cd intent-trading
cp .env.example .env
```

Edit `.env` with production values if deploying (defaults work for local development):

```bash
# Required in production — change these
JWT_SECRET=your-256-bit-secret-here
DATABASE_URL=postgres://user:pass@host:5432/intent_trading
REDIS_URL=redis://host:6379

# Optional — defaults are fine for local dev
SERVER_ADDR=0.0.0.0:3000
GATEWAY_ADDR=0.0.0.0:4000
LOG_LEVEL=info
ENVIRONMENT=dev
```

See [Configuration Guide](configuration.md) for the complete list of 60+ environment variables.

## Step 2: Start Services

```bash
docker compose up -d
```

This starts 13 services:

| Service | Port | What it does |
|---------|------|-------------|
| **postgres** | 5432 | Primary database (39 migrations auto-applied) |
| **redis** | 6379 | Cache, rate limiting, event bus, CSRF tokens |
| **intent-trading** | 3000 | Core platform: API, settlement engine, workers |
| **api-gateway** | 4000 | Auth proxy with JWT validation, rate limiting |
| **solver-bot** | — | Automated solver that bids on intents |
| **frontend** | 3001 | Next.js trading UI |
| **nginx** | 80/443 | Reverse proxy, TLS termination, rate limiting |
| **prometheus** | 9090 | Metrics collection (51 alert rules) |
| **grafana** | 3002 | Dashboards (20 panels) |
| **loki** | 3100 | Log aggregation (7-day retention) |
| **promtail** | — | Ships Docker container logs to Loki |
| **jaeger** | 16686 | Distributed tracing |
| **pg-backup** | — | Daily database backups to S3 (production profile) |

## Step 3: Verify

Wait ~30 seconds for health checks, then:

```bash
# Check all services are running
docker compose ps

# Check platform health
curl -s http://localhost:3000/health/ready | jq
# Expected:
# {
#   "status": "ok",
#   "services": { "db": "ok", "redis": "ok", "engine": "ok" }
# }

# Check API gateway
curl -s http://localhost:4000/health/ready | jq

# Check database has migrations applied
docker compose exec postgres psql -U postgres -d intent_trading -c "SELECT count(*) FROM pg_tables WHERE schemaname = 'public';"
# Should show 15+ tables
```

## Step 4: Create a Test User and Trade

```bash
# Register a user
curl -s -X POST http://localhost:3000/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"testpass123"}' | jq
# → { "token": "eyJ...", "user_id": "...", "email": "...", "roles": ["trader"] }

# Save the token
export TOKEN="<token from above>"
export USER_ID="<user_id from above>"

# Create an account
curl -s -X POST http://localhost:3000/accounts \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"user_id\":\"$USER_ID\"}" | jq
# → { "id": "...", "user_id": "...", "account_type": "spot" }

export ACCOUNT_ID="<id from above>"

# Get a CSRF token (required for mutations via the gateway)
curl -s http://localhost:3000/csrf-token | jq
# → { "token": "..." }

# Deposit funds
curl -s -X POST http://localhost:3000/balances/deposit \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"account_id\":\"$ACCOUNT_ID\",\"asset\":\"USDC\",\"amount\":100000}" | jq

# Submit an intent
curl -s -X POST http://localhost:3000/intents \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"user_id\":\"$USER_ID\",
    \"account_id\":\"$ACCOUNT_ID\",
    \"token_in\":\"USDC\",
    \"token_out\":\"ETH\",
    \"amount_in\":10000,
    \"min_amount_out\":5,
    \"deadline\":$(( $(date +%s) + 3600 ))
  }" | jq
# → { "id": "...", "status": "Open", ... }

# Check your balances
curl -s http://localhost:3000/balances/$ACCOUNT_ID \
  -H "Authorization: Bearer $TOKEN" | jq
# USDC available should have decreased (locked for the intent)
```

## Step 5: Open the UI

Navigate to **http://localhost** (Nginx proxies to the frontend).

1. Login with `test@example.com` / `testpass123`
2. Navigate to a market page
3. You should see the orderbook, trade feed, and intent form

## Running Without Docker (Development)

```bash
# Terminal 1: Start PostgreSQL and Redis
# (install via brew/apt, or use docker for just these two)
docker compose up -d postgres redis

# Terminal 2: Run migrations and start backend
export DATABASE_URL="postgres://postgres:postgres@127.0.0.1:5432/intent_trading"
export REDIS_URL="redis://127.0.0.1:6379"
sqlx database create  # if needed
cargo run              # starts on :3000

# Terminal 3: Start frontend
cd frontend
npm install
npm run dev            # starts on :3000 (Next.js default)

# Terminal 4: Start solver bot (optional)
cargo run --bin solver-bot
```

## Running Tests

```bash
# Fast: unit tests only (296 tests, ~0.3s)
cargo test --bin intent-trading

# Integration tests (requires Docker for testcontainers)
cargo test --features integration

# Frontend E2E (mock API, no backend needed)
cd frontend && npx playwright test

# Solidity
cd contracts && forge test

# Solana programs
cd programs/intentx-htlc && cargo test
cd programs/intentx-settlement && cargo test
```

## Common Issues

### `Failed to start Postgres container` in tests
Integration tests use testcontainers and require Docker. Ensure Docker daemon is running.

### `Connection refused on :3000`
The intent-trading service waits for postgres and redis health checks. Check: `docker compose logs intent-trading`

### `CSRF token missing` on POST requests
Fetch a CSRF token first: `GET /csrf-token`, then include it as `X-CSRF-Token` header.

### `401 Unauthorized`
JWT tokens expire. Re-login via `POST /auth/login` to get a fresh token.

### Frontend shows `OFFLINE`
The WebSocket connection failed. Check that intent-trading is running and that `NEXT_PUBLIC_WS_URL` points to the correct host.

### Grafana shows no data
Prometheus needs a few minutes to scrape metrics. Verify targets at http://localhost:9090/targets — all should show `UP`.
