import React, { useEffect, useState, useCallback } from "react";
import { X, RefreshCw } from "lucide-react";
import { getIntents, cancelIntent, getTradeHistory } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";

type Tab = "open" | "history";

interface Intent {
  id: string;
  user_id: string;
  token_in: string;
  token_out: string;
  amount_in: number;
  min_amount_out: number;
  status: string;
  order_type?: string;
  limit_price?: number;
  stop_price?: number;
  stop_side?: string;
  created_at: number;
}

interface Trade {
  id: string;
  market_id: string;
  price: number;
  qty: number;
  fee: number;
  created_at: string;
}

const STATUS_COLORS: Record<string, string> = {
  Open: "badge-info",
  Bidding: "badge-warning",
  Matched: "badge-success",
  Executing: "bg-purple-500/10 text-purple-400",
  Completed: "badge-success",
  Failed: "badge-danger",
  Cancelled: "bg-surface-3 text-[var(--text-muted)]",
  Expired: "bg-surface-3 text-[var(--text-muted)]",
  PartiallyFilled: "badge-warning",
};

interface OpenOrdersProps {
  marketId: string;
}

const OpenOrders: React.FC<OpenOrdersProps> = ({ marketId }) => {
  const [tab, setTab] = useState<Tab>("open");
  const [intents, setIntents] = useState<Intent[]>([]);
  const [trades, setTrades] = useState<Trade[]>([]);
  const [loading, setLoading] = useState(false);
  const [cancelling, setCancelling] = useState<string | null>(null);
  const { addListener } = useWebSocket();

  const fetchIntents = useCallback(async () => {
    setLoading(true);
    try {
      const data = await getIntents();
      setIntents(data || []);
    } catch {
      setIntents([]);
    }
    setLoading(false);
  }, []);

  const fetchHistory = useCallback(async () => {
    setLoading(true);
    try {
      const data = await getTradeHistory(marketId, 50, 0);
      setTrades(data || []);
    } catch {
      setTrades([]);
    }
    setLoading(false);
  }, [marketId]);

  useEffect(() => {
    if (tab === "open") fetchIntents();
    else fetchHistory();
  }, [tab, fetchIntents, fetchHistory]);

  // Refresh on auction results
  useEffect(() => {
    return addListener("AuctionResult", () => {
      if (tab === "open") fetchIntents();
    });
  }, [tab, addListener, fetchIntents]);

  const openIntents = intents.filter(
    (i) =>
      i.status !== "Completed" &&
      i.status !== "Cancelled" &&
      i.status !== "Failed" &&
      i.status !== "Expired"
  );

  const handleCancel = async (id: string) => {
    setCancelling(id);
    try {
      await cancelIntent(id);
      fetchIntents();
    } catch {}
    setCancelling(null);
  };

  return (
    <div className="rounded-xl border bg-surface-1 overflow-hidden">
      {/* Tab bar */}
      <div className="flex items-center justify-between border-b px-1">
        <div className="flex">
          <button
            onClick={() => setTab("open")}
            className={`px-3 py-2 text-xs font-medium transition-colors border-b-2 ${
              tab === "open"
                ? "border-brand-500 text-[var(--text-primary)]"
                : "border-transparent text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
            }`}
          >
            Open Orders
            {openIntents.length > 0 && (
              <span className="ml-1.5 badge badge-info text-[10px]">
                {openIntents.length}
              </span>
            )}
          </button>
          <button
            onClick={() => setTab("history")}
            className={`px-3 py-2 text-xs font-medium transition-colors border-b-2 ${
              tab === "history"
                ? "border-brand-500 text-[var(--text-primary)]"
                : "border-transparent text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
            }`}
          >
            Order History
          </button>
        </div>
        <button
          onClick={tab === "open" ? fetchIntents : fetchHistory}
          disabled={loading}
          className="btn-ghost !p-1.5 mr-1"
          aria-label="Refresh"
        >
          <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {/* Open orders table */}
      {tab === "open" && (
        <div className="overflow-x-auto">
          <table className="w-full text-[11px]">
            <thead>
              <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                <th className="px-3 py-2 text-left font-medium">Pair</th>
                <th className="px-3 py-2 text-left font-medium">Type</th>
                <th className="px-3 py-2 text-right font-medium">Amount</th>
                <th className="px-3 py-2 text-right font-medium">Min Out</th>
                <th className="px-3 py-2 text-center font-medium">Status</th>
                <th className="px-3 py-2 text-right font-medium">Time</th>
                <th className="px-3 py-2 text-center font-medium"></th>
              </tr>
            </thead>
            <tbody>
              {openIntents.map((intent) => (
                <tr
                  key={intent.id}
                  className="border-t hover:bg-surface-2 transition-colors"
                >
                  <td className="px-3 py-2 font-medium">
                    {intent.token_in}/{intent.token_out}
                  </td>
                  <td className="px-3 py-2 text-[var(--text-muted)]">
                    {intent.order_type || "market"}
                    {intent.stop_price != null && (
                      <span className="text-[10px] ml-1">
                        @{intent.stop_price.toLocaleString()}
                      </span>
                    )}
                    {intent.limit_price != null && (
                      <span className="text-[10px] ml-1">
                        L:{intent.limit_price.toLocaleString()}
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-2 text-right font-mono">
                    {intent.amount_in.toLocaleString()}
                  </td>
                  <td className="px-3 py-2 text-right font-mono">
                    {intent.min_amount_out.toLocaleString()}
                  </td>
                  <td className="px-3 py-2 text-center">
                    <span
                      className={`badge ${STATUS_COLORS[intent.status] || "badge-info"}`}
                    >
                      {intent.status}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-right text-[var(--text-muted)] font-mono">
                    {new Date(intent.created_at * 1000).toLocaleTimeString(
                      [],
                      {
                        hour: "2-digit",
                        minute: "2-digit",
                      }
                    )}
                  </td>
                  <td className="px-3 py-2 text-center">
                    {(intent.status === "Open" ||
                      intent.status === "Bidding") && (
                      <button
                        onClick={() => handleCancel(intent.id)}
                        disabled={cancelling === intent.id}
                        className="btn-ghost !p-1 text-down hover:bg-down/10 rounded"
                        title="Cancel order"
                      >
                        <X size={12} />
                      </button>
                    )}
                  </td>
                </tr>
              ))}
              {openIntents.length === 0 && (
                <tr>
                  <td
                    colSpan={7}
                    className="px-3 py-6 text-center text-[var(--text-muted)]"
                  >
                    No open orders
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}

      {/* Order history table */}
      {tab === "history" && (
        <div className="overflow-x-auto">
          <table className="w-full text-[11px]">
            <thead>
              <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider">
                <th className="px-3 py-2 text-left font-medium">Trade ID</th>
                <th className="px-3 py-2 text-right font-medium">Price</th>
                <th className="px-3 py-2 text-right font-medium">Qty</th>
                <th className="px-3 py-2 text-right font-medium">Fee</th>
                <th className="px-3 py-2 text-right font-medium">Time</th>
              </tr>
            </thead>
            <tbody>
              {trades.map((t) => (
                <tr
                  key={t.id}
                  className="border-t hover:bg-surface-2 transition-colors"
                >
                  <td className="px-3 py-2 font-mono text-[var(--text-muted)]">
                    {t.id.slice(0, 8)}...
                  </td>
                  <td className="px-3 py-2 text-right font-mono text-up">
                    {t.price.toLocaleString()}
                  </td>
                  <td className="px-3 py-2 text-right font-mono">
                    {t.qty.toLocaleString()}
                  </td>
                  <td className="px-3 py-2 text-right font-mono text-[var(--text-muted)]">
                    {t.fee.toLocaleString()}
                  </td>
                  <td className="px-3 py-2 text-right text-[var(--text-muted)]">
                    {new Date(t.created_at).toLocaleString([], {
                      month: "short",
                      day: "numeric",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </td>
                </tr>
              ))}
              {trades.length === 0 && (
                <tr>
                  <td
                    colSpan={5}
                    className="px-3 py-6 text-center text-[var(--text-muted)]"
                  >
                    No trade history
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};

export default OpenOrders;
