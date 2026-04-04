import React, { useEffect, useState, useCallback } from "react";
import { RefreshCw, ChevronLeft, ChevronRight, Filter } from "lucide-react";
import { getMarkets, getTradeHistory } from "@/lib/api";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
}

interface Trade {
  id: string;
  market_id: string;
  buyer_account_id: string;
  seller_account_id: string;
  price: number;
  qty: number;
  fee: number;
  created_at: string;
}

const PAGE_SIZE = 20;

export default function HistoryPage() {
  const [markets, setMarkets] = useState<Market[]>([]);
  const [selectedMarket, setSelectedMarket] = useState("");
  const [trades, setTrades] = useState<Trade[]>([]);
  const [page, setPage] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");

  // Load markets on mount
  useEffect(() => {
    getMarkets()
      .then((data) => {
        setMarkets(data || []);
        if (data?.length > 0) {
          setSelectedMarket(data[0].id);
        }
      })
      .catch(() => {});
  }, []);

  const fetchTrades = useCallback(async () => {
    if (!selectedMarket) return;
    setLoading(true);
    try {
      // Fetch one extra to detect hasMore
      const data = await getTradeHistory(selectedMarket, PAGE_SIZE + 1, page * PAGE_SIZE);
      const items: Trade[] = data || [];
      setHasMore(items.length > PAGE_SIZE);
      setTrades(items.slice(0, PAGE_SIZE));
    } catch {
      setTrades([]);
      setHasMore(false);
    }
    setLoading(false);
  }, [selectedMarket, page]);

  useEffect(() => {
    fetchTrades();
  }, [fetchTrades]);

  // Auto-refresh
  useEffect(() => {
    if (!autoRefresh) return;
    const interval = setInterval(fetchTrades, 5000);
    return () => clearInterval(interval);
  }, [autoRefresh, fetchTrades]);

  // Reset page on market change
  useEffect(() => {
    setPage(0);
  }, [selectedMarket]);

  const marketLabel = (id: string) => {
    const m = markets.find((m) => m.id === id);
    return m ? `${m.base_asset}/${m.quote_asset}` : id.slice(0, 8);
  };

  // Client-side date filter (API doesn't support date range yet)
  const filteredTrades = trades.filter((t) => {
    if (dateFrom && t.created_at < dateFrom) return false;
    if (dateTo && t.created_at > dateTo + "T23:59:59") return false;
    return true;
  });

  return (
    <div className="space-y-6 max-w-6xl mx-auto animate-fade-in">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Trade History</h1>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 text-xs text-[var(--text-muted)] cursor-pointer">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
              className="rounded"
            />
            Auto-refresh
          </label>
          <button
            onClick={fetchTrades}
            disabled={loading}
            className="btn-ghost !p-2"
            aria-label="Refresh"
          >
            <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>

      {/* Filters */}
      <div className="card flex flex-wrap items-end gap-3">
        <div>
          <label className="text-xs text-[var(--text-muted)] block mb-1">
            <Filter size={12} className="inline mr-1" />
            Market
          </label>
          <select
            className="input w-44"
            value={selectedMarket}
            onChange={(e) => setSelectedMarket(e.target.value)}
          >
            {markets.map((m) => (
              <option key={m.id} value={m.id}>
                {m.base_asset}/{m.quote_asset}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-xs text-[var(--text-muted)] block mb-1">From</label>
          <input
            type="date"
            className="input w-36"
            value={dateFrom}
            onChange={(e) => setDateFrom(e.target.value)}
          />
        </div>
        <div>
          <label className="text-xs text-[var(--text-muted)] block mb-1">To</label>
          <input
            type="date"
            className="input w-36"
            value={dateTo}
            onChange={(e) => setDateTo(e.target.value)}
          />
        </div>
        {(dateFrom || dateTo) && (
          <button
            onClick={() => { setDateFrom(""); setDateTo(""); }}
            className="btn-ghost text-xs"
          >
            Clear dates
          </button>
        )}
      </div>

      {/* Table */}
      <div className="card !p-0 overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b text-xs text-[var(--text-muted)] uppercase tracking-wider">
                <th className="px-4 py-3 text-left">Trade ID</th>
                <th className="px-4 py-3 text-left">Market</th>
                <th className="px-4 py-3 text-right">Price</th>
                <th className="px-4 py-3 text-right">Quantity</th>
                <th className="px-4 py-3 text-right">Fee</th>
                <th className="px-4 py-3 text-right">Time</th>
              </tr>
            </thead>
            <tbody>
              {filteredTrades.map((t) => (
                <tr
                  key={t.id}
                  className="border-b border-[var(--border)] hover:bg-surface-2 transition-colors"
                >
                  <td className="px-4 py-2.5 font-mono text-xs text-[var(--text-muted)]">
                    {t.id.slice(0, 8)}...
                  </td>
                  <td className="px-4 py-2.5 font-medium">
                    {marketLabel(t.market_id)}
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono text-up">
                    {t.price.toLocaleString()}
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono">
                    {t.qty.toLocaleString()}
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono text-[var(--text-muted)]">
                    {t.fee.toLocaleString()}
                  </td>
                  <td className="px-4 py-2.5 text-right text-xs text-[var(--text-muted)]">
                    {new Date(t.created_at).toLocaleString()}
                  </td>
                </tr>
              ))}
              {filteredTrades.length === 0 && (
                <tr>
                  <td colSpan={6} className="px-4 py-8 text-center text-[var(--text-muted)]">
                    {loading ? "Loading..." : "No trades found"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {/* Pagination */}
        <div className="flex items-center justify-between px-4 py-3 border-t">
          <span className="text-xs text-[var(--text-muted)]">
            Page {page + 1}
            {filteredTrades.length > 0 &&
              ` \u00B7 ${filteredTrades.length} trade${filteredTrades.length !== 1 ? "s" : ""}`}
          </span>
          <div className="flex gap-1">
            <button
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page === 0}
              className="btn-ghost !p-1.5 disabled:opacity-30"
              aria-label="Previous page"
            >
              <ChevronLeft size={16} />
            </button>
            <button
              onClick={() => setPage((p) => p + 1)}
              disabled={!hasMore}
              className="btn-ghost !p-1.5 disabled:opacity-30"
              aria-label="Next page"
            >
              <ChevronRight size={16} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
