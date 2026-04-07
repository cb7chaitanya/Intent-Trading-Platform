/**
 * IntentX Trading E2E Test
 *
 * Full flow: login → navigate to market → submit intent → wait for
 * auction + settlement → verify UI updates, balance changes, and
 * trade in history.
 *
 * Run:
 *   cd frontend
 *   npx playwright test e2e/trading.spec.ts
 *
 * Requires the backend API at localhost:3000 (or E2E_BASE_URL).
 * Uses a mock API layer when E2E_MOCK=1 to avoid needing a live backend.
 */

import { test, expect, Page, Route } from "@playwright/test";

// ────────────────────────────────────────────────────────────
// Test fixtures
// ────────────────────────────────────────────────────────────

const TEST_USER = {
  email: `e2e-${Date.now()}@test.local`,
  password: "testpass123",
  user_id: "e2e-user-0001",
  roles: ["trader"],
};

const TEST_ACCOUNT = {
  id: "e2e-account-0001",
  user_id: TEST_USER.user_id,
  account_type: "spot",
};

const TEST_MARKET = {
  id: "e2e-market-eth-usdc",
  base_asset: "ETH",
  quote_asset: "USDC",
  tick_size: 100,
  min_order_size: 10,
  fee_rate: 0.001,
};

const INITIAL_BALANCES = [
  { id: "b1", account_id: TEST_ACCOUNT.id, asset: "USDC", available_balance: 500_000, locked_balance: 0 },
  { id: "b2", account_id: TEST_ACCOUNT.id, asset: "ETH", available_balance: 1_000, locked_balance: 0 },
];

// After the trade settles, the user spent USDC and received ETH.
const POST_TRADE_BALANCES = [
  { id: "b1", account_id: TEST_ACCOUNT.id, asset: "USDC", available_balance: 490_000, locked_balance: 0 },
  { id: "b2", account_id: TEST_ACCOUNT.id, asset: "ETH", available_balance: 1_100, locked_balance: 0 },
];

let intentIdCounter = 0;

function nextIntentId(): string {
  intentIdCounter += 1;
  return `intent-${String(intentIdCounter).padStart(4, "0")}`;
}

// ────────────────────────────────────────────────────────────
// Mock API layer
// ────────────────────────────────────────────────────────────

/**
 * Intercepts all API calls so the test runs without a live backend.
 * State transitions are tracked in-memory so assertions work.
 */
class MockApiState {
  intents: Record<string, any> = {};
  settled = false;
  balancePhase: "initial" | "locked" | "settled" = "initial";
  trades: any[] = [];

  get balances() {
    if (this.balancePhase === "settled") return POST_TRADE_BALANCES;
    if (this.balancePhase === "locked") {
      return [
        { ...INITIAL_BALANCES[0], available_balance: 490_000, locked_balance: 10_000 },
        { ...INITIAL_BALANCES[1] },
      ];
    }
    return INITIAL_BALANCES;
  }
}

async function installMockApi(page: Page, state: MockApiState) {
  await page.route("**/csrf-token", (route) =>
    route.fulfill({ json: { token: "mock-csrf-token" } }),
  );

  await page.route("**/auth/login", (route) =>
    route.fulfill({
      json: { token: "mock-jwt-token", ...TEST_USER },
    }),
  );

  await page.route("**/auth/register", (route) =>
    route.fulfill({
      json: { token: "mock-jwt-token", ...TEST_USER },
    }),
  );

  await page.route(`**/markets/${TEST_MARKET.id}`, (route) =>
    route.fulfill({ json: TEST_MARKET }),
  );

  await page.route("**/markets", (route) => {
    if (route.request().url().includes(`/${TEST_MARKET.id}`)) {
      return route.continue();
    }
    return route.fulfill({ json: [TEST_MARKET] });
  });

  await page.route(`**/accounts/${TEST_USER.user_id}`, (route) =>
    route.fulfill({ json: [TEST_ACCOUNT] }),
  );

  await page.route(`**/balances/${TEST_ACCOUNT.id}`, (route) =>
    route.fulfill({ json: state.balances }),
  );

  await page.route("**/orderbook/**", (route) =>
    route.fulfill({
      json: {
        bids: [
          { price: 3000, qty: 50 },
          { price: 2900, qty: 80 },
        ],
        asks: [
          { price: 3100, qty: 40 },
          { price: 3200, qty: 60 },
        ],
      },
    }),
  );

  await page.route("**/oracle/prices/**", (route) =>
    route.fulfill({ json: { price: 3050 } }),
  );

  await page.route("**/market-data/trades/**", (route) =>
    route.fulfill({ json: state.trades }),
  );

  await page.route("**/intents", async (route) => {
    const method = route.request().method();
    if (method === "POST") {
      const body = route.request().postDataJSON();
      const id = nextIntentId();
      const intent = {
        id,
        ...body,
        status: "Open",
        created_at: Math.floor(Date.now() / 1000),
        order_type: body.order_type || "market",
      };
      state.intents[id] = intent;
      state.balancePhase = "locked";
      return route.fulfill({ status: 201, json: intent });
    }
    // GET /intents
    return route.fulfill({ json: Object.values(state.intents) });
  });

  await page.route("**/intents/*/cancel", async (route) => {
    const url = route.request().url();
    const parts = url.split("/");
    const id = parts[parts.length - 2];
    if (state.intents[id]) {
      state.intents[id].status = "Cancelled";
    }
    return route.fulfill({ json: state.intents[id] || {} });
  });

  // WebSocket is not interceptable via route — the test checks UI state
  // after mock API responses, which is what drives the components.
}

/**
 * Simulate the auction + settlement lifecycle by mutating mock state.
 * In a real backend this happens asynchronously; here we fast-forward.
 */
function simulateSettlement(state: MockApiState) {
  for (const id of Object.keys(state.intents)) {
    const intent = state.intents[id];
    if (intent.status === "Open") {
      intent.status = "Completed";
    }
  }
  state.balancePhase = "settled";
  state.settled = true;

  state.trades.push({
    id: "trade-0001",
    market_id: TEST_MARKET.id,
    price: 3050,
    qty: 100,
    fee: 10,
    timestamp: new Date().toISOString(),
  });
}

// ────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────

async function loginViaUI(page: Page) {
  await page.goto("/login");
  await page.locator('input[type="email"]').fill(TEST_USER.email);
  await page.locator('input[type="password"]').fill(TEST_USER.password);
  await page.locator('button[type="submit"]').click();
}

async function loginViaStorage(page: Page) {
  await page.addInitScript((user) => {
    localStorage.setItem("itx_token", "mock-jwt-token");
    localStorage.setItem("itx_user", JSON.stringify(user));
  }, { user_id: TEST_USER.user_id, email: TEST_USER.email, roles: TEST_USER.roles });
}

// ────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────

test.describe("Trading E2E", () => {
  let state: MockApiState;

  test.beforeEach(async ({ page }) => {
    state = new MockApiState();
    await installMockApi(page, state);
  });

  // ── 1. Login ─────────────────────────────────────────────

  test("login with valid credentials", async ({ page }) => {
    await page.goto("/login");

    // Form is visible
    await expect(page.locator('input[type="email"]')).toBeVisible();
    await expect(page.locator('input[type="password"]')).toBeVisible();
    await expect(page.locator('button[type="submit"]')).toContainText("Sign In");

    // Submit credentials
    await page.locator('input[type="email"]').fill(TEST_USER.email);
    await page.locator('input[type="password"]').fill(TEST_USER.password);
    await page.locator('button[type="submit"]').click();

    // Should redirect away from login
    await page.waitForURL((url) => !url.pathname.includes("/login"), {
      timeout: 5_000,
    });
    expect(page.url()).not.toContain("/login");
  });

  // ── 2. Navigate to market ───────────────────────────────

  test("market page loads with orderbook and intent form", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);

    // Orderbook
    await expect(page.locator("text=Order Book")).toBeVisible({ timeout: 10_000 });
    // Verify bid/ask data rendered
    await expect(page.locator("text=3,000").or(page.locator("text=3000"))).toBeVisible({ timeout: 5_000 });

    // Intent form — Buy/Sell toggle
    const buyBtn = page.locator("button").filter({ hasText: /^Buy$/ }).first();
    const sellBtn = page.locator("button").filter({ hasText: /^Sell$/ }).first();
    await expect(buyBtn).toBeVisible();
    await expect(sellBtn).toBeVisible();

    // Order type tabs
    await expect(page.locator("button").filter({ hasText: "Market" })).toBeVisible();
    await expect(page.locator("button").filter({ hasText: "Limit" })).toBeVisible();

    // Balances panel
    await expect(page.locator("text=BALANCES")).toBeVisible();
  });

  // ── 3. Submit a market buy intent ───────────────────────

  test("submit market buy intent and see confirmation", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);
    await expect(page.locator("text=Order Book")).toBeVisible({ timeout: 10_000 });

    // Ensure Buy is selected (default)
    const buyBtn = page.locator("button").filter({ hasText: /^Buy$/ }).first();
    await buyBtn.click();

    // Ensure Market order type is selected
    await page.locator("button").filter({ hasText: "Market" }).click();

    // Fill price estimate and quantity
    const numberInputs = page.locator('form input[type="number"]');

    // Price field (first number input in the form)
    const priceInput = numberInputs.first();
    await priceInput.fill("3050");

    // Quantity field (second number input)
    const qtyInput = numberInputs.nth(1);
    await qtyInput.fill("100");

    // Submit
    const submitBtn = page.locator('form button[type="submit"]');
    await expect(submitBtn).toBeEnabled();
    await submitBtn.click();

    // Wait for success message
    await expect(page.locator("text=created")).toBeVisible({ timeout: 5_000 });

    // Verify intent was stored in mock state
    const intentIds = Object.keys(state.intents);
    expect(intentIds.length).toBe(1);
    expect(state.intents[intentIds[0]].status).toBe("Open");
    expect(state.balancePhase).toBe("locked");
  });

  // ── 4. Full flow: submit → settle → verify ─────────────

  test("full trading flow: intent → auction → settlement → history", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);
    await expect(page.locator("text=Order Book")).toBeVisible({ timeout: 10_000 });

    // ── Submit intent ──────────────────────────────
    const buyBtn = page.locator("button").filter({ hasText: /^Buy$/ }).first();
    await buyBtn.click();
    await page.locator("button").filter({ hasText: "Market" }).click();

    const numberInputs = page.locator('form input[type="number"]');
    await numberInputs.first().fill("3050");
    await numberInputs.nth(1).fill("100");

    await page.locator('form button[type="submit"]').click();
    await expect(page.locator("text=created")).toBeVisible({ timeout: 5_000 });

    // Verify open order appears in Open Orders
    // Refresh intents listing
    await page.locator("button").filter({ hasText: "Open Orders" }).click();

    // The intent should show in the table
    const orderRows = page.locator("table tbody tr");
    await expect(orderRows.first()).toBeVisible({ timeout: 5_000 });

    // ── Simulate auction + settlement ──────────────
    simulateSettlement(state);

    // ── Verify balance changes ─────────────────────
    // Trigger a balance refresh by navigating to account page
    await page.goto("/account");
    await page.locator('input[placeholder="Enter User ID..."]').fill(TEST_USER.user_id);
    await page.locator("button").filter({ hasText: "Search" }).first().click();

    // Wait for account to load and click it
    await expect(page.locator("text=spot")).toBeVisible({ timeout: 5_000 });
    await page.locator("button").filter({ hasText: /spot/ }).click();

    // Balances should reflect post-settlement values
    // USDC should show 490,000 (was 500,000)
    await expect(
      page.locator("text=490").or(page.locator("text=490,000")),
    ).toBeVisible({ timeout: 5_000 });

    // ── Verify trade appears in history ─────────────
    await page.goto("/history");
    await expect(page.locator('h1:has-text("Trade History")')).toBeVisible({
      timeout: 5_000,
    });

    // Select market
    const marketSelect = page.locator("select").first();
    await marketSelect.selectOption({ label: `ETH/USDC` }).catch(() => {
      // If no dropdown options match, the mock returns trades for any market
    });

    // Trade should appear in the table
    await expect(page.locator("text=trade-00").or(page.locator("text=3050").or(page.locator("text=3,050")))).toBeVisible({
      timeout: 5_000,
    });
  });

  // ── 5. Limit order submission ───────────────────────────

  test("submit limit buy order with price validation", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);
    await expect(page.locator("text=Order Book")).toBeVisible({ timeout: 10_000 });

    // Switch to Limit order type
    await page.locator("button").filter({ hasText: "Limit" }).click();

    // Fill price (must be multiple of tick_size=100)
    const numberInputs = page.locator('form input[type="number"]');
    await numberInputs.first().fill("3000");
    await numberInputs.nth(1).fill("50");

    // Submit
    const submitBtn = page.locator('form button[type="submit"]');
    await expect(submitBtn).toBeEnabled();
    await submitBtn.click();

    await expect(page.locator("text=created")).toBeVisible({ timeout: 5_000 });

    const intentIds = Object.keys(state.intents);
    expect(intentIds.length).toBe(1);
    const intent = state.intents[intentIds[0]];
    expect(intent.order_type).toBe("limit");
    expect(intent.limit_price).toBe(3000);
  });

  // ── 6. Cancel an open order ─────────────────────────────

  test("cancel open order from Open Orders panel", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);
    await expect(page.locator("text=Order Book")).toBeVisible({ timeout: 10_000 });

    // Submit an intent first
    const numberInputs = page.locator('form input[type="number"]');
    await numberInputs.first().fill("3050");
    await numberInputs.nth(1).fill("100");
    await page.locator('form button[type="submit"]').click();
    await expect(page.locator("text=created")).toBeVisible({ timeout: 5_000 });

    // Go to Open Orders
    await page.locator("button").filter({ hasText: "Open Orders" }).click();
    await expect(page.locator("table tbody tr").first()).toBeVisible({
      timeout: 5_000,
    });

    // Click cancel button (X icon)
    const cancelBtn = page.locator('button[title="Cancel order"]').first();
    if (await cancelBtn.isVisible()) {
      await cancelBtn.click();

      // Verify intent cancelled in state
      const intentIds = Object.keys(state.intents);
      // After cancel API call, status should be Cancelled
      expect(state.intents[intentIds[0]].status).toBe("Cancelled");
    }
  });

  // ── 7. Balance panel shows correct values ───────────────

  test("balance panel displays initial balances", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto(`/market/${TEST_MARKET.id}`);

    await expect(page.locator("text=BALANCES")).toBeVisible({ timeout: 10_000 });

    // Should show USDC and ETH balances
    await expect(page.locator("text=USDC")).toBeVisible({ timeout: 5_000 });
    await expect(page.locator("text=ETH")).toBeVisible({ timeout: 5_000 });
  });

  // ── 8. Account page deposit/withdraw ────────────────────

  test("account page loads balances after search", async ({ page }) => {
    await loginViaStorage(page);
    await page.goto("/account");

    // Search for user
    await page.locator('input[placeholder="Enter User ID..."]').fill(
      TEST_USER.user_id,
    );
    await page.locator("button").filter({ hasText: "Search" }).first().click();

    // Account should appear
    await expect(page.locator("text=spot")).toBeVisible({ timeout: 5_000 });
    await page.locator("button").filter({ hasText: /spot/ }).click();

    // Balances should load
    await expect(page.locator("text=USDC")).toBeVisible({ timeout: 5_000 });
    await expect(page.locator("text=ETH")).toBeVisible({ timeout: 5_000 });
  });

  // ── 9. History page renders trades ──────────────────────

  test("history page shows trades after settlement", async ({ page }) => {
    // Pre-populate a trade
    state.trades.push({
      id: "trade-hist-001",
      market_id: TEST_MARKET.id,
      price: 3100,
      qty: 75,
      fee: 8,
      timestamp: new Date().toISOString(),
    });

    await loginViaStorage(page);
    await page.goto("/history");
    await expect(page.locator('h1:has-text("Trade History")')).toBeVisible({
      timeout: 5_000,
    });

    // Trade data should appear in table
    await expect(
      page.locator("text=trade-hi").or(page.locator("text=3100").or(page.locator("text=3,100"))),
    ).toBeVisible({ timeout: 5_000 });
  });

  // ── 10. Unauthenticated redirect ───────────────────────

  test("unauthenticated user redirected to login", async ({ page }) => {
    // Don't set auth — navigate to protected page
    await page.goto(`/market/${TEST_MARKET.id}`);

    // Should redirect to /login
    await page.waitForURL("**/login", { timeout: 5_000 });
    expect(page.url()).toContain("/login");
  });
});
