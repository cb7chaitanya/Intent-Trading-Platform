# Solver Bot Guide

The solver bot is a standalone Rust binary (`src/bin/solver_bot.rs`) that connects to the platform via WebSocket, receives new intents, calculates bids, and submits them to compete in auctions. This document covers how it works, how to configure it, and how to reason about the tradeoffs of each bidding strategy.

---

## 1. How It Works

### Connection lifecycle

```
Start → connect WS → listen for events → bid on intents → track positions → loop
  │                                                                          │
  └──── on disconnect: wait 3s → reconnect ─────────────────────────────────┘
```

The bot connects to `WS_URL` (default `ws://127.0.0.1:3000/ws`) and listens for three event types:

| Event | What happens |
|-------|-------------|
| `intent_created` | Bot evaluates the intent and submits a bid via `POST /bids` |
| `intent_matched` | If the bot won, it records the fill in its position tracker |
| `execution_completed` | Logs that settlement finished (informational) |

### Bid submission flow

```
Intent arrives via WS
    │
    ▼
Check position limit
    │ exposure + amount > max_position? → skip
    │
    ▼
Calculate bid based on strategy
    │ amount_out = min_amount_out × premium
    │ fee = amount_in × fee_pct
    │
    ▼
POST /bids { intent_id, solver_id, amount_out, fee }
    │
    ▼
Platform adds bid to auction (10s window)
    │
    ▼
Best bid wins → fill created → settle_fill()
```

The bot submits one bid per intent. It does not adjust bids after submission (no bid amendment during the auction window).

---

## 2. Strategies

### How bids are calculated (`solver_bot.rs:142-147`)

```rust
let (premium_range, fee_pct) = match cfg.bid_strategy {
    Aggressive   => (1.05..1.10, 0.002),
    Conservative => (1.00..1.03, 0.008),
    Balanced     => (1.00..1.10, 0.005),
};

amount_out = min_amount_out × random(premium_range)
fee        = amount_in × fee_pct
```

The `premium_range` is how much **above** the user's `min_amount_out` the bot offers. The `fee_pct` is the solver's take as a percentage of the input amount.

### Strategy comparison

```
                    ┌─────────────┬──────────────┬──────────────┐
                    │ Aggressive  │  Balanced    │ Conservative │
┌───────────────────┼─────────────┼──────────────┼──────────────┤
│ Premium range     │ 1.05–1.10   │ 1.00–1.10    │ 1.00–1.03    │
│ Solver fee        │ 0.2%        │ 0.5%         │ 0.8%         │
│ Win rate          │ Highest     │ Medium       │ Lowest       │
│ Profit per fill   │ Lowest      │ Medium       │ Highest      │
│ Inventory risk    │ Highest     │ Medium       │ Lowest       │
│ Capital needed    │ Most        │ Medium       │ Least        │
└───────────────────┴─────────────┴──────────────┴──────────────┘
```

### Aggressive

```
Premium: 1.05×–1.10× min_amount_out
Fee:     0.2% of amount_in
```

The bot offers 5-10% above the user's minimum. It almost always wins the auction because it gives the user the best deal. But the solver keeps only 0.2% as fee — thin margins.

**When to use**: Market-making with high volume. You win most auctions and make money on volume, not margin. Requires significant capital because you accumulate large positions quickly.

**Example**: User wants to sell 10,000 USDC for at least 5 ETH.
- Aggressive bids: 5.25–5.50 ETH (gives user 5-10% more)
- Fee: 20 USDC (0.2% of 10,000)
- Net cost to solver: 5.25 ETH bought + 20 USDC fee earned
- Profit only if solver can sell the 5.25 ETH above cost

**Risk**: If the market moves against you before you can hedge, losses on the position exceed the 0.2% fee. A 2% adverse move on a 5 ETH position wipes out 25 fills worth of fees.

### Conservative

```
Premium: 1.00×–1.03× min_amount_out
Fee:     0.8% of amount_in
```

The bot offers barely above the user's minimum (0-3% premium). It charges a higher fee (0.8%). It loses most auctions to aggressive solvers, but when it wins, the margin is comfortable.

**When to use**: Low-risk operation. You don't need to win every auction — you only want fills where the economics are clearly favorable. Good for volatile markets where position risk is high.

**Example**: Same intent: sell 10,000 USDC for at least 5 ETH.
- Conservative bids: 5.00–5.15 ETH
- Fee: 80 USDC (0.8% of 10,000)
- Only wins if no other solver offers > 5.15 ETH
- But when it wins, the 80 USDC fee covers a 1.6% adverse move

**Risk**: Low win rate means idle capital. If auctions consistently have aggressive competitors, this bot may go hours without a fill. Reputation score stays low due to low `total_fills`.

### Balanced

```
Premium: 1.00×–1.10× min_amount_out
Fee:     0.5% of amount_in
```

The widest premium range — the bot randomly picks anywhere from the user's minimum to 10% above. The fee is middle-of-road at 0.5%. This creates unpredictable bidding that prevents competitors from systematically undercutting.

**When to use**: General-purpose. Good default for a new solver that doesn't yet know the competitive landscape. The wide range means some bids win (when the random roll is high) and some lose (when the roll is low), creating a natural mix of volume and margin.

**Example**: Same intent.
- Balanced bids: 5.00–5.50 ETH (wide range)
- Fee: 50 USDC (0.5% of 10,000)
- Wins sometimes against aggressive (when roll > 1.05), loses sometimes
- Average win rate depends on competition

---

## 3. Position Management

### Exposure tracking

The bot tracks its total position using the `solver_positions` table in PostgreSQL. Before bidding on any intent, it checks:

```rust
exposure + amount <= max_position
```

Where `exposure = SUM(ABS(position))` across all assets. If the limit is exceeded, the bot logs `Skip {intent_id} — exposure {current}/{max}` and does not bid.

**`MAX_POSITION`** (env var, default: `1,000,000`) is the total absolute position limit in token smallest units. For USDC with 6 decimals, `1,000,000` = $1.00. **Adjust based on your token's decimal precision.**

### Position tracking

When the bot wins an auction (`intent_matched` event), it calls `record_fill()`:

```
New position → INSERT with qty and price
Existing position, same direction → weighted average entry price
Existing position, opposite direction → realize PnL on closed portion, keep avg for remainder
Position goes to zero → mark flat, all PnL realized
```

**Weighted average example**:
```
Fill 1: buy 10 ETH @ 3000 → position=10, avg=3000
Fill 2: buy 5 ETH @ 3200  → position=15, avg=(3000×10 + 3200×5)/15 = 3066
Fill 3: sell 8 ETH @ 3300  → close 8 units, realize 8×(3300-3066)=1872 PnL
                              position=7, avg=3066 (unchanged for remainder)
```

### PnL calculation

```
unrealized_pnl = position × (current_price - avg_entry_price)
total_pnl      = unrealized_pnl + realized_pnl
```

Monitor via the solver dashboard endpoint: `GET /solvers/{id}/dashboard`

---

## 4. Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `SERVER_URL` | `http://127.0.0.1:3000` | Platform API URL |
| `WS_URL` | `ws://127.0.0.1:3000/ws` | WebSocket feed URL |
| `DATABASE_URL` | `postgres://...` | DB for position tracking |
| `API_KEY` | (none) | Optional API key for authenticated requests |
| `SOLVER_ID` | `solver-bot-alpha` | Unique solver identifier |
| `BID_STRATEGY` | `balanced` | `aggressive`, `conservative`, or `balanced` |
| `MAX_POSITION` | `1000000` | Maximum total absolute exposure |
| `POLL_INTERVAL_MS` | `100` | Delay between processing WS events (ms) |

### Running

```bash
# As a Docker service (already in docker-compose.yml)
docker compose up -d solver-bot

# Standalone
BID_STRATEGY=aggressive MAX_POSITION=50000000 cargo run --bin solver-bot

# Multiple solvers with different strategies
SOLVER_ID=solver-aggro BID_STRATEGY=aggressive MAX_POSITION=100000000 cargo run --bin solver-bot &
SOLVER_ID=solver-safe BID_STRATEGY=conservative MAX_POSITION=20000000 cargo run --bin solver-bot &
```

### Registration

Before the bot can bid, the solver must be registered:

```bash
curl -s -X POST http://localhost:3000/solvers/register \
  -H "Content-Type: application/json" \
  -d '{"name":"AlphaSolver","email":"ops@solver.com"}' | jq
# → { "solver_id": "solver-...", "api_key": "itx_...", "name": "AlphaSolver" }
```

Set `SOLVER_ID` and `API_KEY` from the response.

---

## 5. Real-World Scenarios

### Scenario: Stablecoin pair (USDC → USDC cross-chain)

**Market**: Low volatility. Price is always ~1:1 with bridge fees.

**Best strategy**: `aggressive` with tight position limits.

**Reasoning**: Stablecoin swaps have minimal inventory risk. The solver buys USDC on one chain and sells on another — the price barely moves. High win rate via aggressive bidding generates volume-based profit. The 0.2% fee easily covers the bridge cost (~$0.10-$0.50).

**Configuration**:
```bash
BID_STRATEGY=aggressive
MAX_POSITION=500000000  # $500 in 6-decimal USDC
```

### Scenario: Volatile pair (ETH/USDC) during low activity

**Market**: Wide spreads, few solvers, prices can move 5% in minutes.

**Best strategy**: `conservative`.

**Reasoning**: With few competitors, conservative bids still win. The 0.8% fee provides a cushion against the 5% moves. No need to bid aggressively when you're the only solver.

**Configuration**:
```bash
BID_STRATEGY=conservative
MAX_POSITION=10000000  # Keep exposure small
```

### Scenario: ETH/USDC during high activity, many competing solvers

**Market**: Tight spreads, 5+ solvers bidding on every intent.

**Best strategy**: `balanced`.

**Reasoning**: Aggressive might win every auction but with thin margins. Conservative would never win against 5 aggressive bots. Balanced gives a mix — sometimes the random premium roll hits high and wins, sometimes it doesn't. The 0.5% fee is enough to be profitable on average.

**Configuration**:
```bash
BID_STRATEGY=balanced
MAX_POSITION=50000000  # Medium exposure OK with diversified fills
```

### Scenario: New market launch, unknown dynamics

**Best strategy**: Start `conservative`, monitor win rate, switch to `balanced` after 24h of data.

**Monitoring**:
```bash
# Check your win rate
curl -s http://localhost:3000/solvers/$SOLVER_ID/stats | jq '{win_rate, fill_success_rate, total_fills}'

# Check position exposure
curl -s http://localhost:3000/solvers/$SOLVER_ID/dashboard | jq
```

If win rate < 5%: switch to `balanced` or `aggressive`.
If win rate > 80%: you're overpaying. Switch to `conservative` to increase margins.
Target: 20-50% win rate for optimal profit.

---

## 6. Monitoring

### Key metrics

| Metric | Where | What to watch |
|--------|-------|---------------|
| Win rate | `GET /solvers/{id}/stats` → `win_rate` | < 5% = too conservative, > 80% = too aggressive |
| Total fills | `GET /solvers/{id}` → `total_fills` | Growing = healthy, stagnant = not winning |
| Failed fills | `GET /solvers/{id}` → `failed_fills` | > 0 = settlement issues, investigate |
| Reputation score | `GET /solvers/{id}` → `reputation_score` | < 50 = at risk of deprioritization |
| Exposure | Bot startup log: `exposure: X/Y` | Approaching MAX_POSITION = bot will skip intents |

### Alerts to set up

```
If failed_fills increases: settlement is failing for your fills. Check logs.
If exposure hits MAX_POSITION: bot is fully loaded. Increase limit or hedge positions.
If no fills in 1 hour: WebSocket may have disconnected. Check bot logs for "reconnecting".
```

### Log format

```
[solver-bot-alpha] Starting solver bot
[solver-bot-alpha] Positions: 2 assets, exposure: 45000/1000000
[solver-bot-alpha]   ETH: qty=10 avg=3000 rpnl=500
[solver-bot-alpha]   USDC: qty=-30000 avg=1 rpnl=0
[solver-bot-alpha] Connected
[solver-bot-alpha] Bid: id=abc123 out=5250 fee=20
[solver-bot-alpha] WON: pos=15 avg=3066 rpnl=500
[solver-bot-alpha] Skip 7f3a... — exposure 980000/1000000
[solver-bot-alpha] Executed: 7f3a...
```

---

## 7. Limitations

1. **No bid amendment**: Once a bid is submitted, it cannot be updated during the auction. The bot cannot react to other solvers' bids.

2. **Single bid per intent**: The bot submits exactly one bid per intent. It does not submit multiple bids at different price points.

3. **Random premium**: The premium within the range is uniformly random. There is no price discovery or market-making logic. A production solver would use oracle prices, order flow analysis, and inventory-aware pricing.

4. **No hedging**: The bot accumulates positions but does not hedge them. A production solver would hedge on external venues (DEXes, CEXes) immediately after winning an auction.

5. **No cross-chain awareness**: The bot does not differentiate between single-chain and cross-chain intents. Cross-chain intents have additional settlement time and bridge risk that a production solver would price into the bid.

6. **Position tracking is per-asset, not per-pair**: The bot tracks ETH position and USDC position independently. It does not track ETH/USDC pair exposure or cross-asset correlation.
