import React, { useState, useMemo } from "react";
import { ArrowRightLeft, AlertCircle } from "lucide-react";
import { createIntent } from "@/lib/api";

interface IntentFormProps {
  marketId: string;
  balances: Record<string, number>;
  baseAsset?: string;
  quoteAsset?: string;
  tickSize?: number;
  minOrderSize?: number;
}

const TOKENS = ["ETH", "BTC", "SOL", "USDC"];

const IntentForm: React.FC<IntentFormProps> = ({
  marketId,
  balances,
  baseAsset = "ETH",
  quoteAsset = "USDC",
  tickSize = 1,
  minOrderSize = 1,
}) => {
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [userId, setUserId] = useState("");
  const [accountId, setAccountId] = useState("");
  const [tokenIn, setTokenIn] = useState(baseAsset);
  const [tokenOut, setTokenOut] = useState(quoteAsset);
  const [price, setPrice] = useState("");
  const [quantity, setQuantity] = useState("");
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);
  const [loading, setLoading] = useState(false);

  // The asset being spent depends on side
  const spendAsset = side === "buy" ? tokenOut : tokenIn;
  const receiveAsset = side === "buy" ? tokenIn : tokenOut;
  const spendBalance = balances[spendAsset] ?? 0;

  const priceNum = Number(price) || 0;
  const qtyNum = Number(quantity) || 0;
  const total = priceNum * qtyNum;

  // Validation
  const validation = useMemo(() => {
    const errors: string[] = [];

    if (priceNum > 0 && tickSize > 0 && priceNum % tickSize !== 0) {
      errors.push(`Price must be a multiple of tick size (${tickSize})`);
    }
    if (qtyNum > 0 && qtyNum < minOrderSize) {
      errors.push(`Quantity must be at least ${minOrderSize}`);
    }
    if (total > 0 && total > spendBalance) {
      errors.push(
        `Insufficient ${spendAsset} balance (have ${spendBalance.toLocaleString()}, need ${total.toLocaleString()})`
      );
    }

    return errors;
  }, [priceNum, qtyNum, total, tickSize, minOrderSize, spendBalance, spendAsset]);

  const canSubmit =
    userId && accountId && priceNum > 0 && qtyNum > 0 && validation.length === 0 && !loading;

  const handleSwapTokens = () => {
    setTokenIn(tokenOut);
    setTokenOut(tokenIn);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setLoading(true);
    setStatus(null);

    try {
      const deadline = Math.floor(Date.now() / 1000) + 3600;
      const amountIn = side === "buy" ? total : qtyNum;
      const minAmountOut = side === "buy" ? qtyNum : total;

      const intent = await createIntent({
        user_id: userId,
        account_id: accountId,
        token_in: spendAsset,
        token_out: receiveAsset,
        amount_in: amountIn,
        min_amount_out: minAmountOut,
        deadline,
      });
      setStatus({
        type: "success",
        msg: `Intent ${intent.id.slice(0, 8)}... created`,
      });
      setPrice("");
      setQuantity("");
    } catch (err: any) {
      const msg =
        err?.response?.data || err?.message || "Failed to create intent";
      setStatus({ type: "error", msg: String(msg) });
    } finally {
      setLoading(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="card space-y-4">
      {/* Buy / Sell tabs */}
      <div className="flex rounded-lg bg-surface-2 p-1">
        <button
          type="button"
          onClick={() => setSide("buy")}
          className={`flex-1 rounded-md py-2 text-sm font-medium transition-all ${
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
          className={`flex-1 rounded-md py-2 text-sm font-medium transition-all ${
            side === "sell"
              ? "bg-down text-white shadow-sm"
              : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          }`}
        >
          Sell
        </button>
      </div>

      {/* User / Account IDs */}
      <div className="grid grid-cols-2 gap-2">
        <input
          className="input"
          placeholder="User ID"
          value={userId}
          onChange={(e) => setUserId(e.target.value)}
          required
        />
        <input
          className="input"
          placeholder="Account ID"
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
          required
        />
      </div>

      {/* Token pair */}
      <div className="flex items-center gap-2">
        <select
          className="input flex-1"
          value={tokenIn}
          onChange={(e) => setTokenIn(e.target.value)}
        >
          {TOKENS.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <button
          type="button"
          onClick={handleSwapTokens}
          className="btn-ghost !p-2 rounded-full"
          aria-label="Swap tokens"
        >
          <ArrowRightLeft size={16} />
        </button>
        <select
          className="input flex-1"
          value={tokenOut}
          onChange={(e) => setTokenOut(e.target.value)}
        >
          {TOKENS.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
      </div>

      {/* Price */}
      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <label className="text-xs font-medium text-[var(--text-muted)]">
            Price
          </label>
          <span className="text-xs text-[var(--text-muted)]">
            Tick: {tickSize}
          </span>
        </div>
        <input
          type="number"
          className="input font-mono text-lg"
          placeholder="0"
          value={price}
          onChange={(e) => setPrice(e.target.value)}
          step={tickSize}
          min={tickSize}
          required
        />
      </div>

      {/* Quantity */}
      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <label className="text-xs font-medium text-[var(--text-muted)]">
            Quantity
          </label>
          <span className="text-xs text-[var(--text-muted)]">
            Min: {minOrderSize}
          </span>
        </div>
        <input
          type="number"
          className="input font-mono text-lg"
          placeholder="0"
          value={quantity}
          onChange={(e) => setQuantity(e.target.value)}
          min={minOrderSize}
          required
        />
      </div>

      {/* Total + balance */}
      <div className="rounded-lg bg-surface-2 px-3 py-2 space-y-1">
        <div className="flex justify-between text-xs">
          <span className="text-[var(--text-muted)]">Total</span>
          <span className="font-mono font-medium">
            {total > 0 ? total.toLocaleString() : "—"} {spendAsset}
          </span>
        </div>
        <div className="flex justify-between text-xs">
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

      {/* Validation errors */}
      {validation.length > 0 && (priceNum > 0 || qtyNum > 0) && (
        <div className="space-y-1">
          {validation.map((err, i) => (
            <div
              key={i}
              className="flex items-start gap-1.5 text-xs text-down"
            >
              <AlertCircle size={12} className="mt-0.5 shrink-0" />
              <span>{err}</span>
            </div>
          ))}
        </div>
      )}

      <button
        type="submit"
        disabled={!canSubmit}
        className={`w-full py-3 text-sm font-semibold rounded-lg transition-all ${
          side === "buy" ? "btn-success" : "btn-danger"
        }`}
      >
        {loading
          ? "Submitting..."
          : `${side === "buy" ? "Buy" : "Sell"} ${receiveAsset}`}
      </button>

      {status && (
        <div
          className={`rounded-lg px-3 py-2 text-sm animate-slide-up ${
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
