import React, { useEffect, useState, useCallback, useMemo } from "react";
import {
  Clock,
  Play,
  X,
  RefreshCw,
  AlertCircle,
  ChevronDown,
  Layers,
  Timer,
  Target,
} from "lucide-react";
import {
  getMarkets,
  getAccounts,
  getBalances,
  createTwap,
  getActiveTwaps,
  cancelTwap,
} from "@/lib/api";
import { useAuth } from "@/contexts/AuthProvider";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

interface Balance {
  asset: string;
  available_balance: number;
}

interface TwapOrder {
  id: string;
  user_id: string;
  account_id: string;
  token_in: string;
  token_out: string;
  total_qty: number;
  filled_qty: number;
  min_price: number;
  duration_secs: number;
  interval_secs: number;
  slices_total: number;
  slices_completed: number;
  status: string;
  created_at: string;
  finished_at?: string;
}

const DURATION_OPTIONS = [
  { value: 300, label: "5 min" },
  { value: 600, label: "10 min" },
  { value: 900, label: "15 min" },
  { value: 1800, label: "30 min" },
  { value: 3600, label: "1 hour" },
  { value: 7200, label: "2 hours" },
  { value: 14400, label: "4 hours" },
  { value: 28800, label: "8 hours" },
  { value: 86400, label: "24 hours" },
];

const INTERVAL_OPTIONS = [
  { value: 10, label: "10s" },
  { value: 30, label: "30s" },
  { value: 60, label: "1 min" },
  { value: 120, label: "2 min" },
  { value: 300, label: "5 min" },
  { value: 600, label: "10 min" },
  { value: 900, label: "15 min" },
  { value: 1800, label: "30 min" },
];

const STATUS_COLORS: Record<string, string> = {
  Active: "badge-info",
  Completed: "badge-success",
  Cancelled: "bg-surface-3 text-[var(--text-muted)]",
  Failed: "badge-danger",
};

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export default function TwapPage() {
  const { user } = useAuth();

  // Form state
  const [markets, setMarkets] = useState<Market[]>([]);
  const [selectedMarketId, setSelectedMarketId] = useState("");
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [totalQty, setTotalQty] = useState("");
  const [duration, setDuration] = useState(3600);
  const [interval, setInterval_] = useState(60);
  const [limitPrice, setLimitPrice] = useState("");
  const [accountId, setAccountId] = useState("");
  const [balances, setBalances] = useState<Balance[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [submitStatus, setSubmitStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);

  // Active TWAP orders
  const [activeTwaps, setActiveTwaps] = useState<TwapOrder[]>([]);
  const [loadingTwaps, setLoadingTwaps] = useState(false);
  const [cancellingId, setCancellingId] = useState<string | null>(null);

  const selectedMarket = markets.find((m) => m.id === selectedMarketId);
  const baseAsset = selectedMarket?.base_asset || "";
  const quoteAsset = selectedMarket?.quote_asset || "";
  const tokenIn = side === "buy" ? quoteAsset : baseAsset;
  const tokenOut = side === "buy" ? baseAsset : quoteAsset;

  // Load markets
  useEffect(() => {
    getMarkets()
      .then((data) => {
        const items = data || [];
        setMarkets(items);
        if (items.length > 0 && !selectedMarketId) {
          setSelectedMarketId(items[0].id);
        }
      })
      .catch(() => {});
  }, []);

  // Load account + balances
  useEffect(() => {
    if (!user?.user_id) return;
    getAccounts(user.user_id)
      .then((data) => {
        const accts = data || [];
        if (accts.length > 0 && !accountId) {
          setAccountId(accts[0].id);
        }
      })
      .catch(() => {});
  }, [user?.user_id]);

  useEffect(() => {
    if (!accountId) return;
    getBalances(accountId)
      .then((data) => setBalances(data || []))
      .catch(() => setBalances([]));
  }, [accountId]);

  // Load active TWAPs
  const fetchActiveTwaps = useCallback(async () => {
    setLoadingTwaps(true);
    try {
      const data = await getActiveTwaps();
      setActiveTwaps(data || []);
    } catch {
      setActiveTwaps([]);
    }
    setLoadingTwaps(false);
  }, []);

  useEffect(() => {
    fetchActiveTwaps();
  }, [fetchActiveTwaps]);

  // Auto-refresh active TWAPs every 5s
  useEffect(() => {
    const timer = setInterval(fetchActiveTwaps, 5000);
    return () => clearInterval(timer);
  }, [fetchActiveTwaps]);

  // Computed preview values
  const qtyNum = Number(totalQty) || 0;
  const estimatedSlices = interval > 0 ? Math.ceil(duration / interval) : 0;
  const perSlice = estimatedSlices > 0 ? Math.floor(qtyNum / estimatedSlices) : 0;
  const completionTime = new Date(Date.now() + duration * 1000);

  const spendBalance =
    balances.find((b) => b.asset === tokenIn)?.available_balance ?? 0;

  // Validation
  const validation = useMemo(() => {
    const errors: string[] = [];
    if (!selectedMarketId) errors.push("Select a market");
    if (qtyNum <= 0) errors.push("Quantity must be positive");
    if (
      selectedMarket &&
      qtyNum > 0 &&
      qtyNum < selectedMarket.min_order_size
    ) {
      errors.push(`Min order size: ${selectedMarket.min_order_size}`);
    }
    if (interval > duration) errors.push("Interval cannot exceed duration");
    if (estimatedSlices < 2)
      errors.push("At least 2 slices required");
    if (Number(limitPrice) < 0) errors.push("Limit price cannot be negative");
    return errors;
  }, [
    selectedMarketId,
    qtyNum,
    selectedMarket,
    interval,
    duration,
    estimatedSlices,
    limitPrice,
  ]);

  const canSubmit =
    user && accountId && qtyNum > 0 && validation.length === 0 && !submitting;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit || !user) return;
    setSubmitting(true);
    setSubmitStatus(null);
    try {
      const twap = await createTwap({
        user_id: user.user_id,
        account_id: accountId,
        token_in: tokenIn,
        token_out: tokenOut,
        total_qty: qtyNum,
        min_price: Number(limitPrice) || 0,
        duration_secs: duration,
        interval_secs: interval,
      });
      setSubmitStatus({
        type: "success",
        msg: `TWAP order ${twap.id?.slice(0, 8)}... created successfully`,
      });
      setTotalQty("");
      setLimitPrice("");
      fetchActiveTwaps();
    } catch (err: any) {
      const msg = err?.response?.data || err?.message || "Failed to create TWAP";
      setSubmitStatus({ type: "error", msg: String(msg) });
    } finally {
      setSubmitting(false);
    }
  };

  const handleCancel = async (id: string) => {
    setCancellingId(id);
    try {
      await cancelTwap(id);
      fetchActiveTwaps();
    } catch {}
    setCancellingId(null);
  };

  return (
    <div className="space-y-6 max-w-6xl mx-auto animate-fade-in">
      <div className="flex items-center gap-3">
        <div className="h-10 w-10 rounded-xl bg-brand-600/10 flex items-center justify-center">
          <Layers size={20} className="text-brand-400" />
        </div>
        <div>
          <h1 className="text-2xl font-bold">TWAP Orders</h1>
          <p className="text-sm text-[var(--text-muted)]">
            Time-Weighted Average Price execution
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
        {/* ── Submission Form ────────────────────────────── */}
        <form
          onSubmit={handleSubmit}
          className="lg:col-span-5 card space-y-4"
        >
          <h2 className="text-sm font-semibold">New TWAP Order</h2>

          {/* Buy / Sell toggle */}
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

          {/* Market */}
          <div className="space-y-1">
            <label className="text-xs font-medium text-[var(--text-muted)]">
              Market
            </label>
            <div className="relative">
              <select
                className="input !pr-8 appearance-none"
                value={selectedMarketId}
                onChange={(e) => setSelectedMarketId(e.target.value)}
              >
                {markets.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.base_asset}/{m.quote_asset}
                  </option>
                ))}
              </select>
              <ChevronDown
                size={14}
                className="absolute right-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)] pointer-events-none"
              />
            </div>
          </div>

          {/* Total Quantity */}
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-[var(--text-muted)]">
                Total Quantity
              </label>
              <span className="text-[10px] text-[var(--text-muted)]">
                Available: {spendBalance.toLocaleString()} {tokenIn}
              </span>
            </div>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !pr-14"
                placeholder="0"
                value={totalQty}
                onChange={(e) => setTotalQty(e.target.value)}
                min={selectedMarket?.min_order_size || 1}
                required
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-xs text-[var(--text-muted)]">
                {baseAsset}
              </span>
            </div>
          </div>

          {/* Duration */}
          <div className="space-y-1">
            <label className="text-xs font-medium text-[var(--text-muted)] flex items-center gap-1">
              <Clock size={12} />
              Duration
            </label>
            <div className="relative">
              <select
                className="input !pr-8 appearance-none"
                value={duration}
                onChange={(e) => setDuration(Number(e.target.value))}
              >
                {DURATION_OPTIONS.map((d) => (
                  <option key={d.value} value={d.value}>
                    {d.label}
                  </option>
                ))}
              </select>
              <ChevronDown
                size={14}
                className="absolute right-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)] pointer-events-none"
              />
            </div>
          </div>

          {/* Slice Interval */}
          <div className="space-y-1">
            <label className="text-xs font-medium text-[var(--text-muted)] flex items-center gap-1">
              <Timer size={12} />
              Slice Interval
            </label>
            <div className="relative">
              <select
                className="input !pr-8 appearance-none"
                value={interval}
                onChange={(e) => setInterval_(Number(e.target.value))}
              >
                {INTERVAL_OPTIONS.filter((o) => o.value <= duration).map(
                  (o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  )
                )}
              </select>
              <ChevronDown
                size={14}
                className="absolute right-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)] pointer-events-none"
              />
            </div>
          </div>

          {/* Limit Price (optional) */}
          <div className="space-y-1">
            <label className="text-xs font-medium text-[var(--text-muted)] flex items-center gap-1">
              <Target size={12} />
              Limit Price
              <span className="text-[var(--text-muted)] font-normal">
                (optional)
              </span>
            </label>
            <div className="relative">
              <input
                type="number"
                className="input font-mono !pr-14"
                placeholder="No limit"
                value={limitPrice}
                onChange={(e) => setLimitPrice(e.target.value)}
                min={0}
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-xs text-[var(--text-muted)]">
                {quoteAsset}
              </span>
            </div>
          </div>

          {/* ── Estimated Slices Preview ────────────────── */}
          {qtyNum > 0 && estimatedSlices >= 2 && (
            <div className="rounded-lg bg-surface-2 p-3 space-y-2">
              <h3 className="text-xs font-semibold text-[var(--text-secondary)]">
                Preview
              </h3>
              <div className="grid grid-cols-2 gap-y-1.5 text-xs">
                <span className="text-[var(--text-muted)]">Slices</span>
                <span className="text-right font-mono font-medium">
                  {estimatedSlices}
                </span>

                <span className="text-[var(--text-muted)]">Per slice</span>
                <span className="text-right font-mono font-medium">
                  {perSlice.toLocaleString()} {baseAsset}
                </span>

                <span className="text-[var(--text-muted)]">Interval</span>
                <span className="text-right font-mono">
                  {formatDuration(interval)}
                </span>

                <span className="text-[var(--text-muted)]">Duration</span>
                <span className="text-right font-mono">
                  {formatDuration(duration)}
                </span>

                <span className="text-[var(--text-muted)]">
                  Est. completion
                </span>
                <span className="text-right font-mono">
                  {completionTime.toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>

                {Number(limitPrice) > 0 && (
                  <>
                    <span className="text-[var(--text-muted)]">
                      Min price
                    </span>
                    <span className="text-right font-mono">
                      {Number(limitPrice).toLocaleString()} {quoteAsset}
                    </span>
                  </>
                )}
              </div>

              {/* Visual slice preview */}
              <div className="flex gap-0.5 mt-1">
                {Array.from({
                  length: Math.min(estimatedSlices, 40),
                }).map((_, i) => (
                  <div
                    key={i}
                    className={`h-2 flex-1 rounded-sm ${
                      side === "buy" ? "bg-up/30" : "bg-down/30"
                    }`}
                  />
                ))}
                {estimatedSlices > 40 && (
                  <span className="text-[10px] text-[var(--text-muted)] self-center ml-1">
                    +{estimatedSlices - 40}
                  </span>
                )}
              </div>
            </div>
          )}

          {/* Validation errors */}
          {validation.length > 0 && qtyNum > 0 && (
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

          {/* Submit */}
          <button
            type="submit"
            disabled={!canSubmit}
            className={`w-full py-3 text-sm font-semibold rounded-lg transition-all flex items-center justify-center gap-2 ${
              side === "buy" ? "btn-success" : "btn-danger"
            }`}
          >
            {submitting ? (
              <>
                <RefreshCw size={14} className="animate-spin" />
                Submitting...
              </>
            ) : (
              <>
                <Play size={14} />
                {side === "buy" ? "Buy" : "Sell"} {baseAsset} via TWAP
              </>
            )}
          </button>

          {/* Status message */}
          {submitStatus && (
            <div
              className={`rounded-lg px-3 py-2 text-sm animate-slide-up ${
                submitStatus.type === "success"
                  ? "bg-up/10 text-up"
                  : "bg-down/10 text-down"
              }`}
            >
              {submitStatus.msg}
            </div>
          )}
        </form>

        {/* ── Active TWAP Orders ────────────────────────── */}
        <div className="lg:col-span-7 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold">Active TWAP Orders</h2>
            <button
              onClick={fetchActiveTwaps}
              disabled={loadingTwaps}
              className="btn-ghost !p-1.5"
              aria-label="Refresh"
            >
              <RefreshCw
                size={14}
                className={loadingTwaps ? "animate-spin" : ""}
              />
            </button>
          </div>

          {activeTwaps.length === 0 && (
            <div className="card text-center py-12">
              <Layers
                size={28}
                className="mx-auto text-[var(--text-muted)] mb-2"
              />
              <p className="text-sm text-[var(--text-muted)]">
                No active TWAP orders
              </p>
            </div>
          )}

          <div className="space-y-3">
            {activeTwaps.map((twap) => {
              const pct =
                twap.total_qty > 0
                  ? (twap.filled_qty / twap.total_qty) * 100
                  : 0;
              const remaining = twap.total_qty - twap.filled_qty;
              const isActive = twap.status === "Active";
              const isCancelling = cancellingId === twap.id;

              return (
                <div
                  key={twap.id}
                  className="card space-y-3 animate-slide-up"
                >
                  {/* Header */}
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-sm">
                        {twap.token_in} &rarr; {twap.token_out}
                      </span>
                      <span
                        className={`badge ${STATUS_COLORS[twap.status] || "badge-info"}`}
                      >
                        {twap.status}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-mono text-[var(--text-muted)]">
                        {twap.id.slice(0, 8)}...
                      </span>
                      {isActive && (
                        <button
                          onClick={() => handleCancel(twap.id)}
                          disabled={isCancelling}
                          className="btn-ghost !p-1.5 text-down hover:bg-down/10 rounded-lg"
                          title="Cancel TWAP"
                        >
                          {isCancelling ? (
                            <RefreshCw size={14} className="animate-spin" />
                          ) : (
                            <X size={14} />
                          )}
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Progress bar */}
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-xs">
                      <span className="text-[var(--text-muted)]">
                        Progress
                      </span>
                      <span className="font-mono font-medium">
                        {pct.toFixed(1)}%
                      </span>
                    </div>
                    <div className="h-2.5 rounded-full bg-surface-2 overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all duration-500 ${
                          twap.status === "Completed"
                            ? "bg-up"
                            : twap.status === "Cancelled"
                              ? "bg-surface-3"
                              : "bg-brand-500"
                        }`}
                        style={{ width: `${Math.min(pct, 100)}%` }}
                      />
                    </div>
                  </div>

                  {/* Stats grid */}
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                    <div className="rounded-lg bg-surface-2 px-3 py-2">
                      <p className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                        Filled
                      </p>
                      <p className="text-sm font-mono font-medium">
                        {twap.filled_qty.toLocaleString()}
                      </p>
                    </div>
                    <div className="rounded-lg bg-surface-2 px-3 py-2">
                      <p className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                        Remaining
                      </p>
                      <p className="text-sm font-mono font-medium">
                        {remaining.toLocaleString()}
                      </p>
                    </div>
                    <div className="rounded-lg bg-surface-2 px-3 py-2">
                      <p className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                        Slices
                      </p>
                      <p className="text-sm font-mono font-medium">
                        {twap.slices_completed}/{twap.slices_total}
                      </p>
                    </div>
                    <div className="rounded-lg bg-surface-2 px-3 py-2">
                      <p className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                        Interval
                      </p>
                      <p className="text-sm font-mono font-medium">
                        {formatDuration(twap.interval_secs)}
                      </p>
                    </div>
                  </div>

                  {/* Timestamps */}
                  <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                    <span>Created: {formatTime(twap.created_at)}</span>
                    {twap.min_price > 0 && (
                      <span>
                        Limit: {twap.min_price.toLocaleString()}
                      </span>
                    )}
                    {twap.finished_at && (
                      <span>
                        Finished: {formatTime(twap.finished_at)}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
