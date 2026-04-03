import React, { useEffect, useState } from "react";
import { useRouter } from "next/router";
import { getMarket, getIntents } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";
import OrderBook from "@/components/OrderBook";
import TradeFeed from "@/components/TradeFeed";
import IntentForm from "@/components/IntentForm";
import CandlestickChart from "@/components/charts/CandlestickChart";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

interface Intent {
  id: string;
  user_id: string;
  token_in: string;
  token_out: string;
  amount_in: number;
  min_amount_out: number;
  status: string;
  created_at: number;
}

const STATUS_BADGE: Record<string, string> = {
  Open: "badge-info",
  Bidding: "badge-warning",
  Matched: "badge-success",
  Executing: "bg-purple-500/10 text-purple-400",
  Completed: "badge-success",
  Failed: "badge-danger",
  Cancelled: "bg-surface-3 text-[var(--text-muted)]",
};

export default function MarketPage() {
  const router = useRouter();
  const { id } = router.query;
  const marketId = id as string;

  const [market, setMarket] = useState<Market | null>(null);
  const [intents, setIntents] = useState<Intent[]>([]);
  const { subscribe, unsubscribe, connected } = useWebSocket();

  useEffect(() => {
    if (!marketId) return;
    getMarket(marketId)
      .then(setMarket)
      .catch(() => {});
    getIntents()
      .then((data) => setIntents(data || []))
      .catch(() => {});
  }, [marketId]);

  useEffect(() => {
    if (!marketId || !connected) return;
    subscribe(marketId);
    return () => unsubscribe(marketId);
  }, [marketId, connected, subscribe, unsubscribe]);

  if (!marketId) return null;

  const openIntents = intents.filter(
    (i) => i.status !== "Completed" && i.status !== "Cancelled"
  );

  return (
    <div className="space-y-6 max-w-7xl mx-auto animate-fade-in">
      {/* Market header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">
            {market
              ? `${market.base_asset}/${market.quote_asset}`
              : "Loading..."}
          </h1>
          {market && (
            <p className="text-sm text-[var(--text-muted)] mt-0.5">
              Tick {market.tick_size} &middot; Min {market.min_order_size}{" "}
              &middot; Fee {(market.fee_rate * 100).toFixed(2)}%
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span className={`badge ${connected ? "badge-success" : "badge-danger"}`}>
            {connected ? "Live" : "Offline"}
          </span>
        </div>
      </div>

      {/* Chart */}
      <CandlestickChart marketId={marketId} />

      {/* Main grid: OrderBook + Trades | Intent Form + Open Intents */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
        {/* Left: OrderBook + Trades */}
        <div className="lg:col-span-4 space-y-4">
          <OrderBook marketId={marketId} />
        </div>

        <div className="lg:col-span-4 space-y-4">
          <TradeFeed marketId={marketId} />
        </div>

        {/* Right: Intent Form + Open Intents */}
        <div className="lg:col-span-4 space-y-4">
          <IntentForm
            defaultTokenIn={market?.base_asset}
            defaultTokenOut={market?.quote_asset}
          />

          <div className="card space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold">Open Intents</h3>
              <span className="badge badge-info">{openIntents.length}</span>
            </div>

            <div className="space-y-2 max-h-64 overflow-y-auto">
              {openIntents.map((intent) => (
                <div
                  key={intent.id}
                  className="rounded-lg bg-surface-2 p-3 space-y-1 animate-slide-up"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-sm font-medium">
                      {intent.token_in} &rarr; {intent.token_out}
                    </span>
                    <span
                      className={`badge ${STATUS_BADGE[intent.status] || "badge-info"}`}
                    >
                      {intent.status}
                    </span>
                  </div>
                  <div className="flex justify-between text-xs text-[var(--text-muted)]">
                    <span>In: {intent.amount_in.toLocaleString()}</span>
                    <span>
                      Min Out: {intent.min_amount_out.toLocaleString()}
                    </span>
                  </div>
                </div>
              ))}
              {openIntents.length === 0 && (
                <p className="text-center text-sm text-[var(--text-muted)] py-6">
                  No open intents
                </p>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
