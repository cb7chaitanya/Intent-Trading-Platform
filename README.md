# IntentX — Cross-Chain Intent-Based Trading Platform

IntentX is a cross-chain intent-based trading platform where users express **what** they want to trade (intents) rather than **how** to execute it. Solvers compete in real-time auctions to fill intents at the best price, and settlement happens on-chain across Ethereum, Solana, Polygon, Arbitrum, and Base.

```
┌──────────────────────────────────────────────────────────────────────┐
│                        IntentX Architecture                          │
│                                                                      │
│  ┌─────────┐    ┌──────────┐    ┌──────────┐    ┌───────────────┐   │
│  │  Users   │───▶│  Intent  │───▶│ Auction  │───▶│  Settlement   │   │
│  │(Frontend)│    │Submission│    │ Engine   │    │    Engine     │   │
│  └─────────┘    └──────────┘    └──────────┘    └───────┬───────┘   │
│                                       │                  │           │
│                                  ┌────▼────┐    ┌───────▼───────┐   │
│                                  │ Solvers │    │   On-Chain    │   │
│                                  │  (Bots) │    │  Settlement   │   │
│                                  └─────────┘    └───────┬───────┘   │
│                                                         │           │
│                              ┌───────────────┬──────────┴────┐      │
│                              │               │               │      │
│                         ┌────▼───┐    ┌──────▼──┐    ┌──────▼──┐   │
│                         │Ethereum│    │  Solana  │    │Arbitrum │   │
│                         │ (EVM)  │    │(Anchor)  │    │Base/Poly│   │
│                         └────────┘    └─────────┘    └─────────┘   │
│                              │               │               │      │
│                              └───────┬───────┴───────┬───────┘      │
│                                      │               │              │
│                               ┌──────▼──┐    ┌──────▼──────┐       │
│                               │Wormhole │    │  LayerZero   │       │
│                               │ Bridge  │    │   Bridge     │       │
│                               └─────────┘    └─────────────┘       │
└──────────────────────────────────────────────────────────────────────┘
```

## Key Concepts

| Concept | Description |
|---------|-------------|
| **Intent** | A user's trade request: "I want to sell 1000 USDC for at least 0.5 ETH before deadline T" |
| **Solver** | A bot that bids to fill intents. Competes in auctions. Earns fees for execution. |
| **Auction** | 10-second window where solvers submit bids. Best price wins. |
| **Settlement** | On-chain execution of the winning bid. Atomic balance updates via smart contracts. |
| **Cross-chain** | Settlement across different blockchains using Wormhole or LayerZero bridges. |
| **HTLC** | Hash Time-Locked Contract for atomic cross-chain swaps with cryptographic guarantees. |
| **TWAP** | Time-Weighted Average Price orders — large orders split into smaller slices over time. |

## How It Works

```
1. User submits intent    POST /intents { sell 1000 USDC, want ≥ 0.5 ETH, deadline 1h }
2. Auction opens          Solvers see intent via WebSocket, submit bids for 10 seconds
3. Best bid wins          Engine selects highest amount_out bid
4. Settlement executes    Winner's bid → on-chain settlement → balance updates
5. User receives ETH      Double-entry ledger records all movements
```

For cross-chain intents (e.g., ETH on Ethereum → SOL on Solana):

```
1. Lock funds on source chain (Wormhole Token Bridge / LayerZero OFT)
2. Bridge message verified (VAA quorum 13/19 guardians / DVN verification)
3. Release funds on destination chain (completeTransfer / lzReceive)
4. If timeout: automatic refund to source chain
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust (Axum, Tokio, SQLx) — ~75k lines |
| Frontend | Next.js 14, React 18, TypeScript, Tailwind CSS |
| Database | PostgreSQL 16 (39 migrations, partitioned tables) |
| Cache | Redis 7 (rate limiting, CSRF, event bus) |
| EVM Contracts | Solidity 0.8.24 (Foundry, OpenZeppelin) |
| Solana Programs | Anchor (intentx-settlement, intentx-htlc) |
| Bridges | Wormhole Token Bridge, LayerZero v2 OFT |
| Observability | Prometheus, Grafana (20 panels), Loki, Jaeger |
| Infrastructure | Docker Compose (13 services), Nginx, Let's Encrypt |

## Quick Start

**Prerequisites**: Docker and Docker Compose installed.

```bash
# Clone and start
git clone <repo-url> && cd intent-trading
cp .env.example .env
docker compose up -d

# Wait for health checks (~30s)
docker compose ps   # all services should be "healthy" or "running"

# Verify
curl http://localhost:3000/health/ready
# → {"status":"ok","services":{"db":"ok","redis":"ok","engine":"ok"}}

# Open the UI
open http://localhost  # → Nginx proxies to frontend
```

**Access points** after startup:

| Service | URL | Purpose |
|---------|-----|---------|
| Frontend | http://localhost | Trading UI |
| API | http://localhost:3000 | REST API |
| Gateway | http://localhost:4000 | API Gateway (auth proxy) |
| Grafana | http://localhost:3002 | Dashboards (admin/admin) |
| Prometheus | http://localhost:9090 | Metrics |
| Jaeger | http://localhost:16686 | Distributed tracing |

## Project Structure

```
intent-trading/
├── src/                    # Rust backend (~75k lines)
│   ├── main.rs             # App entry point, service wiring
│   ├── api/                # HTTP handlers (intents, bids, orderbook)
│   ├── auth/               # JWT, middleware, key rotation
│   ├── balances/           # Balance management, double-entry ledger
│   ├── cross_chain/        # Wormhole, LayerZero, HTLC, bridge registry
│   ├── settlement/         # Settlement engine, worker, retry
│   ├── engine/             # Auction engine, execution engine
│   ├── wallet/             # Multi-chain signing (ETH, SOL)
│   ├── chaos/              # Fault injection, invariant verification
│   └── ...                 # markets, ws, metrics, config, etc.
├── frontend/               # Next.js frontend
│   ├── pages/              # Routes (market, account, history, etc.)
│   ├── components/         # UI components (orderbook, intent form, etc.)
│   └── e2e/                # Playwright E2E tests
├── contracts/              # Solidity (Foundry)
│   └── src/IntentXSettlement.sol
├── programs/               # Solana Anchor
│   ├── intentx-settlement/
│   └── intentx-htlc/
├── migrations/             # 39 PostgreSQL migrations
├── infra/                  # Prometheus, Grafana, Nginx, Loki, backups
├── tests/                  # Integration + E2E + devnet tests
├── scripts/                # Deployment scripts
└── docs/                   # Documentation
```

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](docs/getting-started.md) | Prerequisites, full setup, verification |
| [Architecture](docs/architecture.md) | System design, intent lifecycle, settlement flows |
| [API Reference](docs/api-reference.md) | All endpoints with request/response examples |
| [Configuration](docs/configuration.md) | All env vars and config.toml settings |
| [Database Schema](docs/database-schema.md) | ERD, migrations, table relationships |
| [Smart Contracts](docs/smart-contracts.md) | Solidity + Anchor program documentation |
| [Cross-Chain Flows](docs/cross-chain.md) | Wormhole, LayerZero, bridge architecture |
| [HTLC Flows](docs/htlc.md) | Atomic swap lifecycle |
| [Frontend Setup](docs/frontend.md) | Next.js development, Playwright tests |
| [Solver Bot Guide](docs/solver-bot.md) | Running and configuring solver bots |
| [Monitoring Runbook](docs/monitoring.md) | Alerts, dashboards, incident response |
| [Deployment Checklist](docs/deployment.md) | Production readiness checklist |
| [Security Audit](docs/security-audit.md) | 51 findings with severity and fix recommendations |
| [Verification Strategy](docs/verification-strategy.md) | Formal invariants, failure matrix, property tests |
| [Backup & Recovery](docs/backup-restore.md) | Database backup, WAL archiving, restoration |

## Tests

```bash
# Unit tests (296 passing)
cargo test --bin intent-trading

# Integration tests (requires Docker)
cargo test --test cross_chain_e2e --features integration
cargo test --test htlc_e2e --features integration
cargo test --test twap_e2e --features integration
cargo test --test chaos_verify --features integration
cargo test --test invariant_proptest --features integration

# Devnet tests (requires funded wallets + env vars)
cargo test --test wormhole_devnet --features devnet

# Frontend E2E tests
cd frontend && npx playwright test

# Solidity tests
cd contracts && forge test

# Solana program tests
cd programs/intentx-htlc && cargo test
cd programs/intentx-settlement && cargo test
```

## License

This project is licensed under the [GNU Affero General Public License v3.0](LICENSE).
