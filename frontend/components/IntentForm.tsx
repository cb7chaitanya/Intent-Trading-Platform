import React, { useState } from "react";
import { ArrowRightLeft } from "lucide-react";
import { createIntent } from "@/lib/api";

interface IntentFormProps {
  defaultTokenIn?: string;
  defaultTokenOut?: string;
}

const TOKENS = ["ETH", "BTC", "SOL", "USDC"];

const IntentForm: React.FC<IntentFormProps> = ({
  defaultTokenIn = "ETH",
  defaultTokenOut = "USDC",
}) => {
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [userId, setUserId] = useState("");
  const [accountId, setAccountId] = useState("");
  const [tokenIn, setTokenIn] = useState(defaultTokenIn);
  const [tokenOut, setTokenOut] = useState(defaultTokenOut);
  const [amountIn, setAmountIn] = useState("");
  const [minAmountOut, setMinAmountOut] = useState("");
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);
  const [loading, setLoading] = useState(false);

  const handleSwapTokens = () => {
    setTokenIn(tokenOut);
    setTokenOut(tokenIn);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setStatus(null);

    try {
      const deadline = Math.floor(Date.now() / 1000) + 3600;
      const intent = await createIntent({
        user_id: userId,
        account_id: accountId,
        token_in: side === "buy" ? tokenOut : tokenIn,
        token_out: side === "buy" ? tokenIn : tokenOut,
        amount_in: Number(amountIn),
        min_amount_out: Number(minAmountOut),
        deadline,
      });
      setStatus({ type: "success", msg: `Intent ${intent.id.slice(0, 8)}... created` });
      setAmountIn("");
      setMinAmountOut("");
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
            <option key={t} value={t}>{t}</option>
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
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
      </div>

      {/* Amount */}
      <div className="space-y-2">
        <label className="text-xs font-medium text-[var(--text-muted)]">
          Amount In
        </label>
        <input
          type="number"
          className="input font-mono text-lg"
          placeholder="0.00"
          value={amountIn}
          onChange={(e) => setAmountIn(e.target.value)}
          required
          min="1"
        />
      </div>

      <div className="space-y-2">
        <label className="text-xs font-medium text-[var(--text-muted)]">
          Min Amount Out
        </label>
        <input
          type="number"
          className="input font-mono text-lg"
          placeholder="0.00"
          value={minAmountOut}
          onChange={(e) => setMinAmountOut(e.target.value)}
          required
          min="1"
        />
      </div>

      <button
        type="submit"
        disabled={loading}
        className={`w-full py-3 text-sm font-semibold rounded-lg transition-all ${
          side === "buy" ? "btn-success" : "btn-danger"
        }`}
      >
        {loading
          ? "Submitting..."
          : `${side === "buy" ? "Buy" : "Sell"} ${tokenIn}`}
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
