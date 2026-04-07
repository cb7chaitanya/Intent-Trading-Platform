# API Reference

Base URL: `http://localhost:3000` (direct) or `http://localhost:4000` (via gateway)

## Authentication

Most endpoints require a JWT token in the `Authorization` header:
```
Authorization: Bearer <token>
```

All mutation requests (POST/PUT/DELETE) require a CSRF token:
```
X-CSRF-Token: <token>
```

Get a CSRF token via `GET /csrf-token`.

---

## Auth

### POST /auth/register

Create a new user account. Returns JWT token immediately.

**Auth required**: No

```json
// Request
{ "email": "user@example.com", "password": "securepass123" }

// Response (201)
{
  "token": "eyJhbGciOiJIUzI1NiJ9...",
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "roles": ["trader"]
}
```

### POST /auth/login

Authenticate and receive a JWT token.

**Auth required**: No

```json
// Request
{ "email": "user@example.com", "password": "securepass123" }

// Response (200)
{
  "token": "eyJhbGciOiJIUzI1NiJ9...",
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "roles": ["trader"]
}
```

### GET /csrf-token

Get a CSRF token for mutation requests. Also sets an HttpOnly cookie.

**Auth required**: No

```json
// Response (200)
{ "token": "a1b2c3d4-e5f6-..." }
```

---

## Accounts

### POST /accounts

Create a trading account for a user.

**Auth required**: Yes

```json
// Request
{ "user_id": "550e8400-..." }

// Response (201)
{
  "id": "660e8400-...",
  "user_id": "550e8400-...",
  "account_type": "spot",
  "created_at": "2026-04-07T12:00:00Z"
}
```

### GET /accounts/{user_id}

List all accounts for a user.

**Auth required**: Yes

```json
// Response (200)
[
  {
    "id": "660e8400-...",
    "user_id": "550e8400-...",
    "account_type": "spot",
    "created_at": "2026-04-07T12:00:00Z"
  }
]
```

---

## Balances

### POST /balances/deposit

Deposit funds into an account. Creates a ledger entry.

**Auth required**: Yes (permission: `balance:deposit`)

```json
// Request
{ "account_id": "660e8400-...", "asset": "USDC", "amount": 100000 }

// Response (200)
{
  "id": "770e8400-...",
  "account_id": "660e8400-...",
  "asset": "USDC",
  "available_balance": 100000,
  "locked_balance": 0,
  "updated_at": "2026-04-07T12:00:00Z"
}
```

**Asset types**: `USDC`, `ETH`, `BTC`, `SOL`

### POST /balances/withdraw

Withdraw funds from an account. Fails if insufficient available balance.

**Auth required**: Yes (permission: `balance:withdraw`)

```json
// Request
{ "account_id": "660e8400-...", "asset": "USDC", "amount": 50000 }

// Response (200)
{
  "id": "770e8400-...",
  "account_id": "660e8400-...",
  "asset": "USDC",
  "available_balance": 50000,
  "locked_balance": 0,
  "updated_at": "2026-04-07T12:01:00Z"
}
```

### GET /balances/{account_id}

Get all balances for an account.

**Auth required**: Yes (permission: `balance:read`)

```json
// Response (200)
[
  { "id": "...", "account_id": "...", "asset": "USDC", "available_balance": 50000, "locked_balance": 10000, "updated_at": "..." },
  { "id": "...", "account_id": "...", "asset": "ETH", "available_balance": 1000, "locked_balance": 0, "updated_at": "..." }
]
```

---

## Intents

### POST /intents

Submit a trading intent. Locks `amount_in` from the user's available balance.

**Auth required**: Yes (permission: `intent:create`)

```json
// Request (market order)
{
  "user_id": "550e8400-...",
  "account_id": "660e8400-...",
  "token_in": "USDC",
  "token_out": "ETH",
  "amount_in": 10000,
  "min_amount_out": 5,
  "deadline": 1712505600
}

// Request (limit order)
{
  "user_id": "550e8400-...",
  "account_id": "660e8400-...",
  "token_in": "USDC",
  "token_out": "ETH",
  "amount_in": 10000,
  "min_amount_out": 5,
  "deadline": 1712505600,
  "order_type": "limit",
  "limit_price": 3000
}

// Request (cross-chain)
{
  "user_id": "550e8400-...",
  "account_id": "660e8400-...",
  "token_in": "ETH",
  "token_out": "SOL",
  "amount_in": 1000,
  "min_amount_out": 500,
  "deadline": 1712505600,
  "source_chain": "ethereum",
  "destination_chain": "solana",
  "cross_chain": true
}

// Response (201)
{
  "id": "880e8400-...",
  "user_id": "550e8400-...",
  "token_in": "USDC",
  "token_out": "ETH",
  "amount_in": 10000,
  "min_amount_out": 5,
  "deadline": 1712505600,
  "status": "Open",
  "created_at": 1712502000,
  "order_type": "market",
  "source_chain": "ethereum",
  "destination_chain": "ethereum",
  "cross_chain": false
}
```

**Intent statuses**: `Open` → `Bidding` → `Matched` → `Executing` → `Completed` | `Failed` | `Cancelled` | `Expired` | `PartiallyFilled`

### GET /intents

List all intents.

**Auth required**: Yes (permission: `intent:read`)

### GET /intents/{id}

Get a specific intent.

**Auth required**: Yes (permission: `intent:read`)

### POST /intents/{id}/cancel

Cancel an open or bidding intent. Unlocks the user's balance.

**Auth required**: Yes

### PUT /intents/{id}/amend

Amend an intent's amount or price. Only works on `Open` or `Bidding` intents. Adjusts locked balance atomically.

**Auth required**: Yes (permission: `intent:create`)

```json
// Request
{
  "account_id": "660e8400-...",
  "amount_in": 15000,
  "min_amount_out": 7,
  "limit_price": 3100
}
```

---

## Bids

### POST /bids

Submit a solver bid for an intent. Solvers compete during the auction window.

**Auth required**: Yes (permission: `bid:create`)

```json
// Request
{
  "intent_id": "880e8400-...",
  "solver_id": "solver-alpha",
  "amount_out": 6,
  "fee": 100
}

// Response (201)
{
  "id": "990e8400-...",
  "intent_id": "880e8400-...",
  "solver_id": "solver-alpha",
  "amount_out": 6,
  "fee": 100,
  "timestamp": 1712502010
}
```

---

## Markets

### GET /markets

List all trading markets.

**Auth required**: No

```json
// Response (200)
[
  {
    "id": "aa0e8400-...",
    "base_asset": "ETH",
    "quote_asset": "USDC",
    "tick_size": 100,
    "min_order_size": 10,
    "fee_rate": 0.001,
    "created_at": "2026-04-07T00:00:00Z"
  }
]
```

### GET /markets/{id}

Get a specific market.

### POST /markets

Create a new market.

```json
// Request
{
  "base_asset": "ETH",
  "quote_asset": "USDC",
  "tick_size": 100,
  "min_order_size": 10,
  "fee_rate": 0.001
}
```

---

## Market Data

### GET /market-data/trades/{market_id}

Get recent trades for a market.

**Query params**: `limit` (default: 100), `offset` (default: 0)

```json
// Response (200)
[
  {
    "id": "bb0e8400-...",
    "market_id": "aa0e8400-...",
    "buyer_account_id": "...",
    "seller_account_id": "...",
    "price": 3050,
    "qty": 100,
    "fee": 10,
    "created_at": "2026-04-07T12:05:00Z"
  }
]
```

### GET /orderbook/{market_id}

Get current orderbook snapshot.

```json
// Response (200)
{
  "market_id": "aa0e8400-...",
  "bids": [{ "price": 3000, "qty": 50 }, { "price": 2900, "qty": 80 }],
  "asks": [{ "price": 3100, "qty": 40 }, { "price": 3200, "qty": 60 }],
  "timestamp": "2026-04-07T12:05:00Z"
}
```

### GET /candles/{market_id}

Get candlestick data.

**Query params**: `interval` (default: `1m`, options: `1m`, `5m`, `15m`, `1h`)

```json
// Response (200)
[
  {
    "market_id": "...",
    "open": 3000,
    "high": 3100,
    "low": 2950,
    "close": 3050,
    "volume": 5000,
    "trade_count": 12,
    "bucket": "2026-04-07T12:00:00Z"
  }
]
```

---

## Oracle

### GET /oracle/prices

Get current prices for all markets.

### GET /oracle/prices/{market_id}

Get current price for a specific market.

```json
// Response (200)
{ "market_id": "...", "price": 3050, "timestamp": "2026-04-07T12:05:00Z" }
```

### GET /oracle/twap/{market_id}

Get time-weighted average price.

**Query params**: `window` (seconds, default: 300, range: 10-86400)

```json
// Response (200)
{ "market_id": "...", "twap_price": 3048, "window": 300, "timestamp": "..." }
```

---

## TWAP

### POST /twap

Create a TWAP order. Splits a large order into time-sliced child intents.

**Auth required**: Yes

```json
// Request
{
  "user_id": "550e8400-...",
  "account_id": "660e8400-...",
  "token_in": "USDC",
  "token_out": "ETH",
  "total_qty": 100000,
  "min_price": 2900,
  "duration_secs": 3600,
  "interval_secs": 60
}

// Response (201)
{
  "id": "cc0e8400-...",
  "slices_total": 60,
  "slices_completed": 0,
  "status": "Active",
  "filled_qty": 0,
  "total_qty": 100000
}
```

### GET /twap/{id}

Get TWAP progress.

```json
// Response (200)
{
  "twap_id": "cc0e8400-...",
  "status": "Active",
  "total_qty": 100000,
  "filled_qty": 35000,
  "slices_total": 60,
  "slices_completed": 21,
  "remaining_qty": 65000,
  "pct_complete": 35.0
}
```

### POST /twap/{id}/cancel

Cancel an active TWAP order.

---

## Solvers

### POST /solvers/register

Register as a solver. Returns an API key for submitting bids.

**Auth required**: No

```json
// Request
{ "name": "AlphaSolver", "email": "ops@solver.com", "webhook_url": "https://..." }

// Response (201)
{ "solver_id": "solver-...", "api_key": "itx_...", "name": "AlphaSolver" }
```

### GET /solvers/top

Get top solvers by reputation score.

**Query params**: `limit` (default: 10)

### GET /solvers/{id}

Get solver public profile.

```json
// Response (200)
{
  "id": "solver-...",
  "name": "AlphaSolver",
  "active": true,
  "successful_trades": 1500,
  "failed_trades": 3,
  "total_volume": 50000000,
  "reputation_score": 98.5,
  "created_at": "2026-01-01T00:00:00Z"
}
```

---

## Health

### GET /health/live

Liveness probe. Returns 200 if the process is running.

### GET /health/ready

Readiness probe. Checks all dependencies.

```json
// Response (200)
{ "status": "ok", "services": { "db": "ok", "redis": "ok", "engine": "ok" } }
```

### GET /health/db

Database health check.

### GET /health/redis

Redis health check.

### GET /metrics

Prometheus metrics endpoint. Returns `text/plain` in Prometheus exposition format.

---

## WebSocket

Connect to `ws://localhost:3000/ws/feed` for real-time updates.

### Subscribe to a market

```json
// Send
{ "action": "subscribe", "market_id": "aa0e8400-..." }

// Receive (confirmation)
{ "type": "Subscribed", "data": { "market_id": "aa0e8400-..." } }
```

### Message types

| Type | Data | When |
|------|------|------|
| `Trade` | `{ id, market_id, price, qty, fee, created_at }` | New trade executed |
| `OrderBook` | `{ bids: [...], asks: [...] }` | Orderbook updated |
| `AuctionResult` | `{ intent_id, winner_id, amount_out }` | Auction completed |

### Unsubscribe

```json
{ "action": "unsubscribe", "market_id": "aa0e8400-..." }
```

---

## Error Responses

All errors follow this format:

```json
// 400 Bad Request
"Insufficient balance"

// 401 Unauthorized
"Missing or invalid token"

// 403 Forbidden
"CSRF token required"

// 404 Not Found
"Intent not found"

// 429 Too Many Requests
"Rate limit exceeded"

// 500 Internal Server Error
"Internal error"
```

## Rate Limits

| Endpoint group | Limit | Window |
|---------------|-------|--------|
| Auth (`/auth/*`) | 5 req/s | Per IP |
| All API (`/api/*`) | 60 req/s | Per IP |
| Intents per user | 30/min | Per user |
| WebSocket connections | No limit | — |
