import axios from "axios";

const api = axios.create({
  baseURL: process.env.NEXT_PUBLIC_API_URL || "http://localhost:3000",
  headers: { "Content-Type": "application/json" },
  withCredentials: true, // send cookies for CSRF
});

// CSRF: fetch token before mutating requests
let csrfToken: string | null = null;

async function ensureCsrfToken() {
  if (!csrfToken) {
    const resp = await api.get("/csrf-token");
    csrfToken = resp.data.token;
  }
  return csrfToken;
}

// Attach CSRF token to POST/PUT/DELETE requests
api.interceptors.request.use(async (config) => {
  const method = config.method?.toUpperCase();
  if (method === "POST" || method === "PUT" || method === "DELETE" || method === "PATCH") {
    const token = await ensureCsrfToken();
    if (token) {
      config.headers["X-CSRF-Token"] = token;
    }
  }
  return config;
});

// If CSRF token is rejected, refresh and retry once
api.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (error.response?.status === 403 && error.response?.data?.includes?.("CSRF")) {
      csrfToken = null;
      const token = await ensureCsrfToken();
      if (token && error.config) {
        error.config.headers["X-CSRF-Token"] = token;
        return api.request(error.config);
      }
    }
    return Promise.reject(error);
  }
);

// Auth
export const authRegister = (data: { email: string; password: string }) =>
  api.post("/auth/register", data).then((r) => r.data);

export const authLogin = (data: { email: string; password: string }) =>
  api.post("/auth/login", data).then((r) => r.data);

// Markets
export const getMarkets = () => api.get("/markets").then((r) => r.data);
export const getMarket = (id: string) =>
  api.get(`/markets/${id}`).then((r) => r.data);

// Intents
export const createIntent = (data: {
  user_id: string;
  account_id: string;
  token_in: string;
  token_out: string;
  amount_in: number;
  min_amount_out: number;
  deadline: number;
}) => api.post("/intents", data).then((r) => r.data);

export const getIntents = () => api.get("/intents").then((r) => r.data);
export const getIntent = (id: string) =>
  api.get(`/intents/${id}`).then((r) => r.data);

// Bids
export const submitBid = (data: {
  intent_id: string;
  solver_id: string;
  amount_out: number;
  fee: number;
}) => api.post("/bids", data).then((r) => r.data);

// Orderbook
export const getOrderbook = (marketId: string) =>
  api.get(`/orderbook/${marketId}`).then((r) => r.data);

// Trades
export const getTrades = (marketId: string, limit = 100) =>
  api
    .get(`/market-data/trades/${marketId}`, { params: { limit } })
    .then((r) => r.data);

export const getTradeHistory = (marketId: string, limit = 50, offset = 0) =>
  api
    .get(`/market-data/trades/${marketId}`, { params: { limit, offset } })
    .then((r) => r.data);

// Candles
export const getCandles = (marketId: string, interval = "1m") =>
  api
    .get(`/candles/${marketId}`, { params: { interval } })
    .then((r) => r.data);

// Balances
export const getBalances = (accountId: string) =>
  api.get(`/balances/${accountId}`).then((r) => r.data);

export const deposit = (data: {
  account_id: string;
  asset: string;
  amount: number;
}) => api.post("/balances/deposit", data).then((r) => r.data);

export const withdraw = (data: {
  account_id: string;
  asset: string;
  amount: number;
}) => api.post("/balances/withdraw", data).then((r) => r.data);

// Accounts
export const getAccounts = (userId: string) =>
  api.get(`/accounts/${userId}`).then((r) => r.data);

export const createAccount = (data: { user_id: string }) =>
  api.post("/accounts", data).then((r) => r.data);

// Amend Intent
export const amendIntent = (
  id: string,
  data: {
    account_id: string;
    amount_in?: number;
    min_amount_out?: number;
    limit_price?: number;
  }
) => api.put(`/intents/${id}/amend`, data).then((r) => r.data);

// Cancel Intent (set status to cancelled)
export const cancelIntent = (id: string) =>
  api.post(`/intents/${id}/cancel`).then((r) => r.data);

// TWAP
export const createTwap = (data: {
  user_id: string;
  account_id: string;
  token_in: string;
  token_out: string;
  total_qty: number;
  min_price: number;
  duration_secs: number;
  interval_secs: number;
}) => api.post("/twap", data).then((r) => r.data);

export const getTwapProgress = (twapId: string) =>
  api.get(`/twap/${twapId}`).then((r) => r.data);

export const getActiveTwaps = () =>
  api.get("/twap/active").then((r) => r.data);

export const cancelTwap = (twapId: string) =>
  api.post(`/twap/${twapId}/cancel`).then((r) => r.data);

// Oracle
export const getOraclePrices = () =>
  api.get("/oracle/prices").then((r) => r.data);

export const getOraclePrice = (marketId: string) =>
  api.get(`/oracle/prices/${marketId}`).then((r) => r.data);

// Solvers
export const getTopSolvers = (limit = 10) =>
  api.get("/solvers/top", { params: { limit } }).then((r) => r.data);

export const getSolver = (id: string) =>
  api.get(`/solvers/${id}`).then((r) => r.data);

export default api;
