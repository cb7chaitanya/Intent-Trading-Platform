import React, { useState, useMemo, useEffect } from "react";
import { AlertCircle, ArrowDownUp } from "lucide-react";
import {
  createCrossChainIntent,
  getAccounts,
  getBalances,
} from "@/lib/api";
import { useAuth } from "@/contexts/AuthProvider";
import ChainSelector, { SUPPORTED_CHAINS } from "./ChainSelector";
import RouteInfo from "./RouteInfo";
import SettlementTracker from "./SettlementTracker";

interface Balance {
  asset: string;
  available_balance: number;
  locked_balance: number;
}

interface CrossChainFormProps {
  baseAsset?: string;
  quoteAsset?: string;
  onOrderPlaced?: () => void;
}

const CrossChainForm: React.FC<CrossChainFormProps> = ({
  baseAsset = "ETH",
  quoteAsset = "USDC",
  onOrderPlaced,
}) => {
  const { user } = useAuth();

  // Chain selection
  const [sourceChain, setSourceChain] = useState("ethereum");
  const [destChain, setDestChain] = useState("solana");

  // Form fields
  const [tokenIn, setTokenIn] = useState(quoteAsset);
  const [tokenOut, setTokenOut] = useState(baseAsset);
  const [amount, setAmount] = useState("");
  const [minAmountOut, setMinAmountOut] = useState("");

  // Account / balance
  const [accountId, setAccountId] = useState("");
  const [accounts, setAccounts] = useState<{ id: string }[]>([]);
  const [balances, setBalances] = useState<Balance[]>([]);

  // Status
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);
  const [loading, setLoading] = useState(false);
  const [submittedIntentId, setSubmittedIntentId] = useState<string | null>(
    null
  );

  // Load accounts
  useEffect(() => {
    if (!user) return;
    getAccounts(user.user_id)
      .then((accts) => {
        setAccounts(accts);
        if (accts.length > 0 && !accountId) setAccountId(accts[0].id);
      })
      .catch(() => {});
  }, [user]);

  // Load balances when account changes
  useEffect(() => {
    if (!accountId) return;
    getBalances(accountId)
      .then(setBalances)
      .catch(() => setBalances([]));
  }, [accountId]);

  // Computed values
  const amountNum = Number(amount) || 0;
  const minOutNum = Number(minAmountOut) || 0;

  const spendBalance = useMemo(() => {
    const b = balances.find(
      (b) => b.asset.toUpperCase() === tokenIn.toUpperCase()
    );
    return b?.available_balance || 0;
  }, [balances, tokenIn]);

  // Validation
  const errors = useMemo(() => {
    const errs: string[] = [];
    if (sourceChain === destChain) errs.push("Source and destination must differ");
    if (amountNum <= 0) errs.push("Amount must be positive");
    if (minOutNum <= 0) errs.push("Min amount out must be positive");
    if (amountNum > spendBalance) errs.push("Insufficient balance");
    return errs;
  }, [sourceChain, destChain, amountNum, minOutNum, spendBalance]);

  const canSubmit =
    user && accountId && errors.length === 0 && !loading && amountNum > 0;

  // Handlers
  const handleSwapChains = () => {
    setSourceChain(destChain);
    setDestChain(sourceChain);
    setTokenIn(tokenOut);
    setTokenOut(tokenIn);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit || !user) return;

    setLoading(true);
    setStatus(null);
    setSubmittedIntentId(null);

    try {
      const deadline = Math.floor(Date.now() / 1000) + 3600;

      const intent = await createCrossChainIntent({
        user_id: user.user_id,
        account_id: accountId,
        token_in: tokenIn,
        token_out: tokenOut,
        amount_in: amountNum,
        min_amount_out: minOutNum,
        deadline,
        source_chain: sourceChain,
        destination_chain: destChain,
      });

      setStatus({
        type: "success",
        msg: `Cross-chain order ${intent.id?.slice(0, 8)}... created`,
      });
      setSubmittedIntentId(intent.id);
      setAmount("");
      setMinAmountOut("");
      onOrderPlaced?.();

      // Refresh balances
      if (accountId) {
        getBalances(accountId)
          .then(setBalances)
          .catch(() => {});
      }
    } catch (err: any) {
      const msg = err?.response?.data || err?.message || "Order failed";
      setStatus({ type: "error", msg: String(msg) });
    } finally {
      setLoading(false);
    }
  };

  const availableTokens = ["USDC", "ETH", "BTC", "SOL"];

  return (
    <div className="flex flex-col h-full rounded-xl border bg-surface-1 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b bg-surface-2">
        <div className="flex items-center gap-2">
          <ArrowDownUp size={14} className="text-brand-400" />
          <span className="text-xs font-semibold">Cross-Chain</span>
        </div>
      </div>

      <form onSubmit={handleSubmit} className="flex-1 flex flex-col p-3 gap-3 overflow-y-auto">
        {/* Chain selector */}
        <ChainSelector
          sourceChain={sourceChain}
          destChain={destChain}
          onSourceChange={setSourceChain}
          onDestChange={setDestChain}
          onSwap={handleSwapChains}
        />

        {/* Route info */}
        <RouteInfo
          sourceChain={sourceChain}
          destChain={destChain}
          token={tokenIn}
          amount={amountNum}
        />

        {/* Token selection */}
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
              Send
            </label>
            <select
              value={tokenIn}
              onChange={(e) => setTokenIn(e.target.value)}
              className="input w-full text-sm"
            >
              {availableTokens.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
              Receive
            </label>
            <select
              value={tokenOut}
              onChange={(e) => setTokenOut(e.target.value)}
              className="input w-full text-sm"
            >
              {availableTokens
                .filter((t) => t !== tokenIn)
                .map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
            </select>
          </div>
        </div>

        {/* Amount */}
        <div>
          <div className="flex items-center justify-between mb-1">
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
              Amount
            </label>
            <span className="text-[10px] text-[var(--text-muted)]">
              Avail: <span className="font-mono">{spendBalance.toLocaleString()}</span>
            </span>
          </div>
          <input
            type="number"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder="0"
            min="1"
            className="input font-mono"
          />
          {/* Percentage buttons */}
          <div className="grid grid-cols-4 gap-1 mt-1.5">
            {[25, 50, 75, 100].map((pct) => (
              <button
                key={pct}
                type="button"
                onClick={() =>
                  setAmount(String(Math.floor(spendBalance * (pct / 100))))
                }
                className="rounded bg-surface-2 py-1 text-[10px] font-medium text-[var(--text-muted)] hover:bg-surface-3"
              >
                {pct}%
              </button>
            ))}
          </div>
        </div>

        {/* Min amount out */}
        <div>
          <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
            Min Receive
          </label>
          <input
            type="number"
            value={minAmountOut}
            onChange={(e) => setMinAmountOut(e.target.value)}
            placeholder="0"
            min="1"
            className="input font-mono"
          />
        </div>

        {/* Account selector (if multiple) */}
        {accounts.length > 1 && (
          <div>
            <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
              Account
            </label>
            <select
              value={accountId}
              onChange={(e) => setAccountId(e.target.value)}
              className="input w-full text-sm"
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.id.slice(0, 8)}...
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Validation errors */}
        {errors.length > 0 && amountNum > 0 && (
          <div className="rounded-lg bg-down/10 px-3 py-2 text-[11px] text-down flex items-start gap-1.5">
            <AlertCircle size={12} className="shrink-0 mt-0.5" />
            <div className="space-y-0.5">
              {errors.map((e, i) => (
                <div key={i}>{e}</div>
              ))}
            </div>
          </div>
        )}

        {/* Submit */}
        <button
          type="submit"
          disabled={!canSubmit}
          className="w-full py-2.5 text-xs font-semibold rounded-lg btn-primary"
        >
          {loading ? "Submitting..." : `Bridge ${tokenIn} → ${tokenOut}`}
        </button>

        {/* Status message */}
        {status && (
          <div
            className={`rounded-lg px-3 py-2 text-[11px] animate-slide-up ${
              status.type === "success"
                ? "bg-up/10 text-up"
                : "bg-down/10 text-down"
            }`}
          >
            {status.msg}
          </div>
        )}

        {/* Settlement tracker for submitted intent */}
        {submittedIntentId && (
          <SettlementTracker intentId={submittedIntentId} />
        )}
      </form>
    </div>
  );
};

export default CrossChainForm;
