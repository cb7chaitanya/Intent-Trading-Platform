import React, { useState, useMemo, useEffect } from "react";
import { AlertCircle, Clock, ArrowDownUp } from "lucide-react";
import { createIntent, createTwap, getAccounts, getBalances } from "@/lib/api";
import { useAuth } from "@/contexts/AuthProvider";

type OrderType = "market" | "limit" | "stop" | "twap";

interface IntentFormProps {
  marketId: string;
  baseAsset?: string;
  quoteAsset?: string;
  tickSize?: number;
  minOrderSize?: number;
  onOrderPlaced?: () => void;
  initialPrice?: number;
}

interface Balance {
  asset: string;
  available_balance: number;
  locked_balance: number;
}

const ORDER_TYPES: { value: OrderType; label: string }[] = [
  { value: "market", label: "Market" },
  { value: "limit", label: "Limit" },
  { value: "stop", label: "Stop" },
  { value: "twap", label: "TWAP" },
];

const IntentForm: React.FC<IntentFormProps> = ({
  marketId,
  baseAsset = "ETH",
  quoteAsset = "USDC",
  tickSize = 1,
  minOrderSize = 1,
  onOrderPlaced,
  initialPrice,
}) => {
  const { user } = useAuth();

  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [orderType, setOrderType] = useState<OrderType>("market");
  const [price, setPrice] = useState("");
  const [quantity, setQuantity] = useState("");
  const [stopPrice, setStopPrice] = useState("");
  const [limitPrice, setLimitPrice] = useState("");

  // TWAP fields
  const [twapDuration, setTwapDuration] = useState("3600"); // seconds
  const [twapInterval, setTwapInterval] = useState("60"); // seconds
  const [twapMinPrice, setTwapMinPrice] = useState("");

  const [accountId, setAccountId] = useState("");
  const [accounts, setAccounts] = useState<{ id: string }[]>([]);
  const [balances, setBalances] = useState<Balance[]>([]);
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);
  const [loading, setLoading] = useState(false);

  // Auto-load accounts when user is available
  useEffect(() => {
    if (!user?.user_id) return;
    getAccounts(user.user_id)
      .then((data) => {
        const accts = data || [];
        setAccounts(accts);
        if (accts.length > 0 && !accountId) {
          setAccountId(accts[0].id);
        }
      })
      .catch(() => {});
  }, [user?.user_id]);

  // Load balances when account changes
  useEffect(() => {
    if (!accountId) return;
    getBalances(accountId)
      .then((data) => setBalances(data || []))
      .catch(() => setBalances([]));
  }, [accountId]);

  // Set price from external click (orderbook/trade feed)
  useEffect(() => {
    if (initialPrice != null && initialPrice > 0) {
      setPrice(String(initialPrice));
    }
  }, [initialPrice]);

  const spendAsset = side === "buy" ? quoteAsset : baseAsset;
  const receiveAsset = side === "buy" ? baseAsset : quoteAsset;
  const spendBalance =
    balances.find((b) => b.asset === spendAsset)?.available_balance ?? 0;

  const priceNum = Number(price) || 0;
  const qtyNum = Number(quantity) || 0;
  const total = priceNum * qtyNum;

  const validation = useMemo(() => {
    const errors: string[] = [];
    if (orderType !== "market" && priceNum > 0 && tickSize > 0 && priceNum % tickSize !== 0) {
      errors.push(`Price must be a multiple of tick size (${tickSize})`);
    }
    if (qtyNum > 0 && qtyNum < minOrderSize) {
      errors.push(`Min order size: ${minOrderSize}`);
    }
    if (total > 0 && spendBalance > 0 && total > spendBalance) {
      errors.push(`Insufficient ${spendAsset} (${spendBalance.toLocaleString()} available)`);
    }
    if (orderType === "stop" && !stopPrice) {
      errors.push("Stop price required");
    }
    if (orderType === "twap") {
      const dur = Number(twapDuration);
      const intv = Number(twapInterval);
      if (dur <= 0) errors.push("Duration must be positive");
      if (intv <= 0) errors.push("Interval must be positive");
      if (intv > dur) errors.push("Interval cannot exceed duration");
    }
    return errors;
  }, [
    orderType, priceNum, qtyNum, total, tickSize, minOrderSize,
    spendBalance, spendAsset, stopPrice, twapDuration, twapInterval,
  ]);

  const canSubmit =
    user &&
    accountId &&
    qtyNum > 0 &&
    (orderType === "market" || priceNum > 0) &&
    validation.length === 0 &&
    !loading;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit || !user) return;
    setLoading(true);
    setStatus(null);

    try {
      const deadline = Math.floor(Date.now() / 1000) + 3600;

      if (orderType === "twap") {
        const twap = await createTwap({
          user_id: user.user_id,
          account_id: accountId,
          token_in: spendAsset,
          token_out: receiveAsset,
          total_qty: qtyNum,
          min_price: Number(twapMinPrice) || 0,
          duration_secs: Number(twapDuration),
          interval_secs: Number(twapInterval),
        });
        setStatus({
          type: "success",
          msg: `TWAP ${twap.id?.slice(0, 8)}... created`,
        });
      } else {
        const amountIn = side === "buy" ? total : qtyNum;
        const minAmountOut = side === "buy" ? qtyNum : total;

        const payload: any = {
          user_id: user.user_id,
          account_id: accountId,
          token_in: spendAsset,
          token_out: receiveAsset,
          amount_in: amountIn,
          min_amount_out: minAmountOut,
          deadline,
        };

        // For limit/stop orders, include extra fields
        if (orderType === "limit") {
          payload.order_type = "limit";
          payload.limit_price = priceNum;
        } else if (orderType === "stop") {
          payload.order_type = "stop";
          payload.stop_price = Number(stopPrice);
          payload.stop_side = side;
          if (limitPrice) {
            payload.limit_price = Number(limitPrice);
          }
        }

        const intent = await createIntent(payload);
        setStatus({
          type: "success",
          msg: `${orderType.charAt(0).toUpperCase() + orderType.slice(1)} order ${intent.id?.slice(0, 8)}... created`,
        });
      }

      setQuantity("");
      onOrderPlaced?.();
    } catch (err: any) {
      const msg = err?.response?.data || err?.message || "Order failed";
      setStatus({ type: "error", msg: String(msg) });
    } finally {
      setLoading(false);
    }
  };

  const pctButtons = [25, 50, 75, 100];

  const handlePct = (pct: number) => {
    if (spendBalance <= 0 || priceNum <= 0) return;
    const spendAmt = Math.floor(spendBalance * (pct / 100));
    const qty = side === "buy" ? Math.floor(spendAmt / priceNum) : spendAmt;
    setQuantity(String(qty));
  };

  return (
    <form
      onSubmit={handleSubmit}
      className="flex flex-col h-full rounded-xl border bg-surface-1 overflow-hidden"
    >
      {/* Buy / Sell toggle */}
      <div className="flex p-1.5 border-b">
        <button
          type="button"
          onClick={() => setSide("buy")}
          className={`flex-1 rounded-lg py-2 text-xs font-semibold transition-all ${
            side === "buy"
              ? "bg-up text-white shadow-sm"
              : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          }`}
        >
          Buy
        </button>
        <button
          type="button"
          onClick={() => setSide("sell")}
          className={`flex-1 rounded-lg py-2 text-xs font-semibold transition-all ${
            side === "sell"
              ? "bg-down text-white shadow-sm"
              : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          }`}
        >
          Sell
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {/* Order type tabs */}
        <div className="flex rounded-lg bg-surface-2 p-0.5">
          {ORDER_TYPES.map((ot) => (
            <button
              key={ot.value}
              type="button"
              onClick={() => setOrderType(ot.value)}
              className={`flex-1 rounded-md py-1.5 text-[11px] font-medium transition-all ${
                orderType === ot.value
                  ? "bg-surface-1 text-[var(--text-primary)] shadow-sm"
                  : "text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
              }`}
            >
              {ot.label}
            </button>
          ))}
        </div>

        {/* Account selector */}
        {accounts.length > 1 && (
          <select
            className="input !py-1.5 !text-xs"
            value={accountId}
            onChange={(e) => setAccountId(e.target.value)}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.id.slice(0, 12)}...
              </option>
            ))}
          </select>
        )}

        {/* Price (not shown for market orders) */}
        {orderType !== "market" && orderType !== "twap" && (
          <div className="space-y-1">
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
              {orderType === "stop" ? "Mark Price" : "Price"}
            </label>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !text-sm !pr-14"
                placeholder="0"
                value={price}
                onChange={(e) => setPrice(e.target.value)}
                step={tickSize}
                min={0}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                {quoteAsset}
              </span>
            </div>
          </div>
        )}

        {/* Stop price */}
        {orderType === "stop" && (
          <div className="space-y-1">
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
              Stop Price
            </label>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !text-sm !pr-14"
                placeholder="0"
                value={stopPrice}
                onChange={(e) => setStopPrice(e.target.value)}
                step={tickSize}
                min={0}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                {quoteAsset}
              </span>
            </div>
          </div>
        )}

        {/* Stop-limit price (optional) */}
        {orderType === "stop" && (
          <div className="space-y-1">
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
              Limit Price (optional)
            </label>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !text-sm !pr-14"
                placeholder="Market on trigger"
                value={limitPrice}
                onChange={(e) => setLimitPrice(e.target.value)}
                step={tickSize}
                min={0}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                {quoteAsset}
              </span>
            </div>
          </div>
        )}

        {/* Market price input for pricing estimation */}
        {orderType === "market" && (
          <div className="space-y-1">
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
              Est. Price
            </label>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !text-sm !pr-14"
                placeholder="For estimation"
                value={price}
                onChange={(e) => setPrice(e.target.value)}
                min={0}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                {quoteAsset}
              </span>
            </div>
          </div>
        )}

        {/* Quantity */}
        {orderType !== "twap" && (
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
                Quantity
              </label>
              <span className="text-[10px] text-[var(--text-muted)]">
                Min: {minOrderSize}
              </span>
            </div>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !text-sm !pr-14"
                placeholder="0"
                value={quantity}
                onChange={(e) => setQuantity(e.target.value)}
                min={minOrderSize}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                {baseAsset}
              </span>
            </div>
            {/* Percentage buttons */}
            <div className="grid grid-cols-4 gap-1">
              {pctButtons.map((pct) => (
                <button
                  key={pct}
                  type="button"
                  onClick={() => handlePct(pct)}
                  className="rounded bg-surface-2 py-1 text-[10px] font-medium text-[var(--text-muted)] hover:bg-surface-3 hover:text-[var(--text-secondary)] transition-colors"
                >
                  {pct}%
                </button>
              ))}
            </div>
          </div>
        )}

        {/* TWAP-specific fields */}
        {orderType === "twap" && (
          <>
            <div className="space-y-1">
              <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
                Total Quantity
              </label>
              <div className="relative">
                <input
                  type="number"
                  className="input font-mono !text-sm !pr-14"
                  placeholder="0"
                  value={quantity}
                  onChange={(e) => setQuantity(e.target.value)}
                  min={minOrderSize}
                />
                <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                  {baseAsset}
                </span>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1">
                <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider flex items-center gap-1">
                  <Clock size={10} />
                  Duration
                </label>
                <select
                  className="input !text-xs"
                  value={twapDuration}
                  onChange={(e) => setTwapDuration(e.target.value)}
                >
                  <option value="900">15 min</option>
                  <option value="1800">30 min</option>
                  <option value="3600">1 hour</option>
                  <option value="7200">2 hours</option>
                  <option value="14400">4 hours</option>
                  <option value="28800">8 hours</option>
                  <option value="86400">24 hours</option>
                </select>
              </div>
              <div className="space-y-1">
                <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
                  Interval
                </label>
                <select
                  className="input !text-xs"
                  value={twapInterval}
                  onChange={(e) => setTwapInterval(e.target.value)}
                >
                  <option value="30">30s</option>
                  <option value="60">1 min</option>
                  <option value="120">2 min</option>
                  <option value="300">5 min</option>
                  <option value="600">10 min</option>
                  <option value="900">15 min</option>
                </select>
              </div>
            </div>

            <div className="space-y-1">
              <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
                Min Price (optional)
              </label>
              <div className="relative">
                <input
                  type="number"
                  className="input font-mono !text-sm !pr-14"
                  placeholder="No minimum"
                  value={twapMinPrice}
                  onChange={(e) => setTwapMinPrice(e.target.value)}
                  min={0}
                />
                <span className="absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-[var(--text-muted)]">
                  {quoteAsset}
                </span>
              </div>
            </div>

            {/* TWAP summary */}
            {qtyNum > 0 && Number(twapDuration) > 0 && Number(twapInterval) > 0 && (
              <div className="rounded-lg bg-surface-2 px-3 py-2 space-y-1 text-[11px]">
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">Slices</span>
                  <span className="font-mono">
                    {Math.ceil(Number(twapDuration) / Number(twapInterval))}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[var(--text-muted)]">Per slice</span>
                  <span className="font-mono">
                    {Math.floor(
                      qtyNum /
                        Math.ceil(
                          Number(twapDuration) / Number(twapInterval)
                        )
                    ).toLocaleString()}{" "}
                    {baseAsset}
                  </span>
                </div>
              </div>
            )}
          </>
        )}

        {/* Order summary */}
        {orderType !== "twap" && total > 0 && (
          <div className="rounded-lg bg-surface-2 px-3 py-2 space-y-1">
            <div className="flex justify-between text-[11px]">
              <span className="text-[var(--text-muted)]">Total</span>
              <span className="font-mono font-medium">
                {total.toLocaleString()} {quoteAsset}
              </span>
            </div>
            <div className="flex justify-between text-[11px]">
              <span className="text-[var(--text-muted)]">Available</span>
              <span
                className={`font-mono ${
                  total > spendBalance
                    ? "text-down font-medium"
                    : "text-[var(--text-secondary)]"
                }`}
              >
                {spendBalance.toLocaleString()} {spendAsset}
              </span>
            </div>
          </div>
        )}

        {/* Validation errors */}
        {validation.length > 0 && (priceNum > 0 || qtyNum > 0) && (
          <div className="space-y-1">
            {validation.map((err, i) => (
              <div
                key={i}
                className="flex items-start gap-1.5 text-[11px] text-down"
              >
                <AlertCircle size={11} className="mt-0.5 shrink-0" />
                <span>{err}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Submit button */}
      <div className="p-3 pt-0">
        <button
          type="submit"
          disabled={!canSubmit}
          className={`w-full py-2.5 text-xs font-semibold rounded-lg transition-all ${
            side === "buy" ? "btn-success" : "btn-danger"
          }`}
        >
          {loading
            ? "Submitting..."
            : `${side === "buy" ? "Buy" : "Sell"} ${receiveAsset}${
                orderType !== "market" ? ` (${orderType.toUpperCase()})` : ""
              }`}
        </button>
      </div>

      {/* Status message */}
      {status && (
        <div
          className={`mx-3 mb-3 rounded-lg px-3 py-2 text-[11px] animate-slide-up ${
            status.type === "success"
              ? "bg-up/10 text-up"
              : "bg-down/10 text-down"
          }`}
        >
          {status.msg}
        </div>
      )}
    </form>
  );
};

export default IntentForm;
