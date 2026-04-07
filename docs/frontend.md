# Frontend Reference

Next.js 14, React 18, TypeScript, Tailwind CSS. Communicates with the backend via REST (Axios) and WebSocket (native). Integrates Phantom wallet for Solana on-chain deposits.

---

## 1. Architecture

```
_app.tsx
├── SolanaProvider          ← Phantom wallet, always mounted
│   └── AuthProvider        ← JWT session, axios interceptors
│       └── AppContent
│           ├── /login, /register  → no layout, no WS
│           └── all other pages
│               └── WebSocketProvider  ← live data feed
│                   └── Layout (Sidebar + Topbar)
│                       └── Page component
```

**Provider mount order matters**. SolanaProvider wraps everything because the wallet button appears in the Topbar even on auth pages. AuthProvider wraps AppContent because it controls the redirect-to-login logic. WebSocketProvider only mounts on authenticated pages to avoid connecting before the user has logged in.

---

## 2. Backend Communication

### REST API (Axios)

**Client**: `frontend/lib/api.ts`

```typescript
const api = axios.create({
  baseURL: process.env.NEXT_PUBLIC_API_URL || "http://localhost:3000",
  headers: { "Content-Type": "application/json" },
  withCredentials: true,  // sends cookies for CSRF
});
```

All API functions are thin wrappers:
```typescript
export const getMarkets = () => api.get("/markets").then(r => r.data);
export const createIntent = (data) => api.post("/intents", data).then(r => r.data);
```

**CSRF protection**: The client auto-fetches a CSRF token before any POST/PUT/DELETE:

```
1. First mutation request → interceptor calls GET /csrf-token
2. Server returns { token: "..." } and sets HttpOnly cookie
3. Client attaches X-CSRF-Token header to all subsequent mutations
4. If server returns 403 with "CSRF" in body → refresh token, retry once
```

The CSRF token is cached in a module-level variable (`let csrfToken`). It persists for the tab's lifetime unless invalidated.

### WebSocket

**Client**: `frontend/contexts/WebSocketProvider.tsx`

```
URL: process.env.NEXT_PUBLIC_WS_URL || "ws://localhost:3000/ws/feed"
```

**Connection lifecycle**:
```
Mount → connect() → onopen → re-subscribe to pending markets
                  → onmessage → dispatch to listeners
                  → onclose → backoff → reconnect
                  → onerror → close → reconnect
```

**Reconnect strategy**: Exponential backoff with jitter.
```
delay = min(1000ms × 2^retries + random(0-500ms), 30000ms)
```
Retries reset to 0 on successful connection.

**Subscribing to a market**:
```typescript
const { subscribe, unsubscribe } = useWebSocket();

// In market page useEffect:
subscribe(marketId);  // sends { action: "subscribe", market_id: "..." }
return () => unsubscribe(marketId);
```

Subscriptions are tracked in `pendingSubscriptions` ref. On reconnect, all pending subscriptions are re-sent automatically.

**Message types received**:

| Type | Data shape | What it updates |
|------|-----------|-----------------|
| `Trade` | `{ id, market_id, price, qty, fee, created_at }` | `trades` state (prepend, max 200) |
| `OrderBook` | `{ bids: [{price, qty}], asks: [{price, qty}] }` | `orderbook` state (replace) |
| `AuctionResult` | `{ intent_id, winner_id, amount_out }` | Dispatched to listeners |
| `CrossChainUpdate` | `{ intent_id, status, ... }` | Dispatched to listeners |

**Custom listeners**: Components subscribe to specific message types:
```typescript
const { addListener } = useWebSocket();
useEffect(() => {
  return addListener("AuctionResult", (data) => {
    // refresh open orders
  });
}, [addListener]);
```

`addListener` returns an unsubscribe function.

---

## 3. Authentication Flow

### Login

```
User submits email + password
    │
    ▼
POST /auth/login { email, password }
    │
    ▼
Server returns { token, user_id, email, roles }
    │
    ▼
Client stores in localStorage:
    itx_token = "eyJhbG..."
    itx_user  = '{"user_id":"...","email":"...","roles":["trader"]}'
    │
    ▼
Axios interceptor attaches Authorization: Bearer <token> to all requests
    │
    ▼
Router navigates to /
```

### Session restore

On page load, `AuthProvider` checks `localStorage` for `itx_token` and `itx_user`. If both exist and the token parses, the session is restored without a network call. No token validation is done client-side — the first API call will return 401 if the token is expired, which triggers logout.

### Token expiry

The response interceptor watches for 401:

```typescript
if (error.response?.status === 401) {
  localStorage.removeItem("itx_token");
  localStorage.removeItem("itx_user");
  router.push("/login");
}
```

There is no proactive token refresh. The user must re-login when the JWT expires.

### Route protection

`AuthProvider` redirects unauthenticated users to `/login` for all routes except `/login` and `/register`:

```typescript
if (!loading && !user && !PUBLIC_PATHS.includes(router.pathname)) {
  router.push("/login");
}
```

### Role checking

```typescript
const { hasRole } = useAuth();
if (hasRole("admin")) { /* show admin panel */ }
```

`hasRole` returns `true` if `user.roles` includes the requested role OR includes `"admin"` (admin has all roles).

---

## 4. Solana Wallet Integration

### Provider stack

```typescript
// _app.tsx
<SolanaProvider>      ← ConnectionProvider + WalletProvider + WalletModalProvider
  <AuthProvider>
    ...
```

**Supported wallets**: Phantom (`PhantomWalletAdapter`). Others can be added to the `wallets` array in `SolanaProvider.tsx:26`.

**Network selection**:
```
NEXT_PUBLIC_SOLANA_NETWORK = "devnet" | "mainnet-beta"  (default: devnet)
NEXT_PUBLIC_SOLANA_RPC_URL = custom endpoint             (optional, overrides cluster default)
```

The endpoint is computed once via `useMemo`:
```typescript
const endpoint = CUSTOM_RPC || clusterApiUrl(NETWORK);
```

### On-chain deposit flow

The `SolanaDeposit` component (`components/solana-wallet/SolanaDeposit.tsx`) handles SPL token deposits into the settlement program's vault:

```
User enters amount
    │
    ▼
Build transaction:
    1. Check if user's Associated Token Account exists
       └── If not: add createAssociatedTokenAccountInstruction
    2. Build deposit instruction:
       - Compute config PDA: seeds=[b"config"]
       - Compute user account PDA: seeds=[b"user", wallet, mint]
       - Anchor discriminator: SHA-256("global:deposit")[0..8]
       - Data: discriminator(8B) + amount(8B LE)
    3. Set recent blockhash + fee payer
    │
    ▼
signTransaction(tx)      ← Phantom popup for approval
    │
    ▼
connection.sendRawTransaction(signed)
    │
    ▼
connection.confirmTransaction(sig, "confirmed")
    │
    ▼
Display success with Solana Explorer link
```

**PDA derivation** (must match on-chain program):
```typescript
// Config: seeds = [b"config"]
PublicKey.findProgramAddressSync([Buffer.from("config")], PROGRAM_ID)

// User account: seeds = [b"user", owner, mint]
PublicKey.findProgramAddressSync([Buffer.from("user"), owner.toBuffer(), mint.toBuffer()], PROGRAM_ID)

// Vault authority: seeds = [b"vault", config]
PublicKey.findProgramAddressSync([Buffer.from("vault"), config.toBuffer()], PROGRAM_ID)
```

### Wallet button

The `WalletButton` component (`components/solana-wallet/WalletButton.tsx`) renders the Solana wallet-adapter-react-ui connect/disconnect button in the Topbar. It uses the `@solana/wallet-adapter-react-ui` styles imported in `_app.tsx`.

---

## 5. Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `NEXT_PUBLIC_API_URL` | `http://localhost:3000` | Backend REST API base URL. In Docker: set to `/api` (proxied by Nginx). |
| `NEXT_PUBLIC_WS_URL` | `ws://localhost:3000/ws/feed` | WebSocket feed URL. In Docker: `wss://localhost/ws/feed` (via Nginx). |
| `NEXT_PUBLIC_SOLANA_NETWORK` | `devnet` | Solana cluster. Set to `mainnet-beta` for production. |
| `NEXT_PUBLIC_SOLANA_RPC_URL` | (cluster default) | Custom Solana RPC endpoint. Overrides the cluster default URL. |

All `NEXT_PUBLIC_*` variables are embedded at build time by Next.js. Changing them requires a rebuild (`npm run build`).

**Docker Compose values** (`docker-compose.yml`):
```yaml
frontend:
  environment:
    - NEXT_PUBLIC_API_URL=/api          # Nginx proxies /api/ → api-gateway:4000
    - NEXT_PUBLIC_WS_URL=wss://localhost/ws/feed  # Nginx proxies /ws/ → intent-trading:3000
```

---

## 6. Local Development Setup

### Prerequisites

- Node.js 20+
- npm 9+
- Backend running on `localhost:3000` (or Docker)

### Steps

```bash
cd frontend
npm install

# Option A: backend via Docker (recommended)
# In project root:
docker compose up -d postgres redis intent-trading

# Option B: backend running locally
# Ensure cargo run is active in another terminal

# Start frontend dev server
npm run dev
# → http://localhost:3000 (Next.js default)
```

**Port conflict**: Next.js defaults to port 3000, which conflicts with the Rust backend. Solutions:

```bash
# Option 1: use a different port for Next.js
PORT=3001 npm run dev

# Option 2: use the Docker setup (Nginx routes traffic)
docker compose up -d
# Frontend at http://localhost (port 80)
```

### Playwright E2E tests

```bash
# Install browsers (first time)
npx playwright install chromium

# Run tests (mock API, no backend needed)
npm run test:e2e

# Interactive mode
npm run test:e2e:ui
```

The Playwright tests use `page.route()` to intercept all API calls with mock responses. They do not require a running backend.

---

## 7. Project Structure

```
frontend/
├── pages/
│   ├── _app.tsx            ← Provider stack, layout routing
│   ├── login.tsx           ← Email + password form
│   ├── register.tsx        ← Registration form
│   ├── index.tsx           ← Dashboard
│   ├── markets.tsx         ← Market list
│   ├── market/[id].tsx     ← Trading page (orderbook, chart, intent form)
│   ├── account.tsx         ← Balance management
│   ├── history.tsx         ← Trade history with pagination
│   ├── twap.tsx            ← TWAP order management
│   ├── leaderboard.tsx     ← Solver leaderboard
│   └── admin.tsx           ← Admin panel (admin role required)
├── components/
│   ├── layout/             ← Layout, Sidebar, Topbar
│   ├── intent-form/        ← IntentForm (market/limit/stop/TWAP)
│   ├── cross-chain/        ← CrossChainForm, ChainSelector, RouteInfo, SettlementTracker
│   ├── orderbook/          ← OrderBook display
│   ├── trade-feed/         ← Recent trades
│   ├── chart/              ← Candlestick chart (lightweight-charts)
│   ├── balances/           ← BalancesPanel
│   ├── open-orders/        ← OpenOrders (with cross-chain tracker)
│   ├── solana-wallet/      ← WalletButton, SolanaDeposit
│   └── market-selector/    ← MarketSelector dropdown
├── contexts/
│   ├── AuthProvider.tsx    ← JWT session, axios interceptors
│   ├── WebSocketProvider.tsx ← WS connection, subscriptions, listeners
│   └── SolanaProvider.tsx  ← Phantom wallet adapter
├── lib/
│   ├── api.ts              ← Axios client, all API functions, CSRF
│   ├── ws.ts               ← (unused, functionality in WebSocketProvider)
│   ├── solana.ts           ← PDA derivation, deposit/withdraw instructions
│   └── solana-config.ts    ← Program IDs (generated by anchor_deploy.sh)
├── hooks/
│   └── useTheme.ts         ← Dark/light mode toggle
├── styles/
│   └── globals.css         ← CSS variables, component classes, Tailwind
├── e2e/
│   └── trading.spec.ts     ← Playwright E2E tests (10 tests)
├── playwright.config.ts
├── tailwind.config.js
├── tsconfig.json
└── package.json
```

---

## 8. Common Issues

### `CORS error` when calling API

The backend must allow the frontend's origin. In local dev, both are on `localhost` but different ports. The Axios client sets `withCredentials: true`, so the backend must respond with:
```
Access-Control-Allow-Origin: http://localhost:3001
Access-Control-Allow-Credentials: true
```

**Fix**: Use Docker Compose with Nginx — it proxies both frontend and API from the same origin, eliminating CORS.

### `WebSocket connection failed`

The WebSocket URL must match the backend's WS endpoint. Common issues:

```
Wrong: ws://localhost:3000/ws          ← missing /feed
Right: ws://localhost:3000/ws/feed

Wrong: ws://localhost:4000/ws/feed     ← gateway doesn't proxy WS
Right: ws://localhost:3000/ws/feed     ← direct to intent-trading

Docker: wss://localhost/ws/feed        ← Nginx handles TLS + proxy
```

### `401 Unauthorized` on every request

JWT token expired or localStorage corrupted. Clear and re-login:
```javascript
localStorage.removeItem("itx_token");
localStorage.removeItem("itx_user");
// Refresh the page → redirects to /login
```

### `403 Forbidden` on POST requests

Missing CSRF token. The axios interceptor should auto-fetch it, but if Redis is down the token can't be stored/validated server-side.

**Check**: Is Redis running? `docker compose exec redis redis-cli ping`

### Solana wallet won't connect

- Phantom extension must be installed
- Network must match: if frontend is configured for devnet but wallet is on mainnet, transactions will fail
- Check browser console for adapter errors

### `ReferenceError: localStorage is not defined`

Happens during server-side rendering (SSR). The `AuthProvider` accesses `localStorage` in a `useEffect` (client-side only), so this should not occur in normal operation. If it does, a component is reading auth state during SSR.

**Fix**: Wrap localStorage access in `typeof window !== 'undefined'` checks.

### Build fails with `Module not found: solana-config`

The file `lib/solana-config.ts` is generated by `scripts/anchor_deploy.sh`. If you haven't deployed Solana programs, create a placeholder:

```typescript
// frontend/lib/solana-config.ts
export const SOLANA_CONFIG = {
  settlementProgramId: "11111111111111111111111111111111",
  htlcProgramId: "11111111111111111111111111111111",
  network: "devnet",
  rpcUrl: "https://api.devnet.solana.com",
};
```
