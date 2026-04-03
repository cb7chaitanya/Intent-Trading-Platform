import React, { useEffect, useState, useRef } from "react";
import { getTrades } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";

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

interface TradeFeedProps {
  marketId: string;
}

const TradeFeed: React.FC<TradeFeedProps> = ({ marketId }) => {
  const [trades, setTrades] = useState<Trade[]>([]);
  const [flashId, setFlashId] = useState<string | null>(null);
  const { addListener } = useWebSocket();

  useEffect(() => {
    getTrades(marketId, 50)
      .then((data) => setTrades(data || []))
      .catch(() => {});
  }, [marketId]);

  useEffect(() => {
    return addListener("Trade", (data) => {
      if (data.market_id === marketId) {
        setTrades((prev) => [data, ...prev].slice(0, 50));
        setFlashId(data.id);
        setTimeout(() => setFlashId(null), 600);
      }
    });
  }, [marketId, addListener]);

  return (
    <div className="card space-y-3">
      <h3 className="text-sm font-semibold">Recent Trades</h3>

      <div className="grid grid-cols-3 text-xs text-[var(--text-muted)] px-2">
        <span>Price</span>
        <span className="text-right">Size</span>
        <span className="text-right">Time</span>
      </div>

      <div className="max-h-64 overflow-y-auto space-y-px">
        {trades.map((t) => (
          <div
            key={t.id}
            className={`grid grid-cols-3 px-2 py-1 text-xs rounded-sm transition-colors hover:bg-surface-2 ${
              t.id === flashId ? "animate-flash-green" : ""
            }`}
          >
            <span className="text-up font-mono">{t.price}</span>
            <span className="text-right font-mono">{t.qty}</span>
            <span className="text-right text-[var(--text-muted)] font-mono">
              {new Date(t.created_at).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
                second: "2-digit",
              })}
            </span>
          </div>
        ))}
        {trades.length === 0 && (
          <p className="text-center text-sm text-[var(--text-muted)] py-6">
            No trades yet
          </p>
        )}
      </div>
    </div>
  );
};

export default TradeFeed;
