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
  onPriceClick?: (price: number) => void;
  maxItems?: number;
}

const TradeFeed: React.FC<TradeFeedProps> = ({
  marketId,
  onPriceClick,
  maxItems = 40,
}) => {
  const [trades, setTrades] = useState<Trade[]>([]);
  const [flashId, setFlashId] = useState<string | null>(null);
  const prevPriceRef = useRef<number | null>(null);
  const { addListener } = useWebSocket();

  useEffect(() => {
    getTrades(marketId, maxItems)
      .then((data) => {
        const items = data || [];
        setTrades(items);
        if (items.length > 0) prevPriceRef.current = items[0].price;
      })
      .catch(() => {});
  }, [marketId, maxItems]);

  useEffect(() => {
    return addListener("Trade", (data) => {
      if (data.market_id === marketId) {
        setTrades((prev) => [data, ...prev].slice(0, maxItems));
        setFlashId(data.id);
        prevPriceRef.current = data.price;
        setTimeout(() => setFlashId(null), 600);
      }
    });
  }, [marketId, addListener, maxItems]);

  const getSideColor = (trade: Trade, index: number) => {
    // Compare with next trade (older) to determine if price went up or down
    const next = trades[index + 1];
    if (!next) return "text-up";
    return trade.price >= next.price ? "text-up" : "text-down";
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-3 py-2 border-b">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--text-muted)]">
          Recent Trades
        </h3>
      </div>

      {/* Header */}
      <div className="grid grid-cols-3 px-2 py-1.5 text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
        <span>Price</span>
        <span className="text-right">Size</span>
        <span className="text-right">Time</span>
      </div>

      {/* Trade list */}
      <div className="flex-1 overflow-y-auto">
        {trades.map((t, i) => (
          <div
            key={t.id}
            onClick={() => onPriceClick?.(t.price)}
            className={`grid grid-cols-3 px-2 py-[3px] text-[11px] cursor-pointer hover:bg-surface-2 transition-colors ${
              t.id === flashId ? "animate-flash-green" : ""
            }`}
          >
            <span className={`font-mono ${getSideColor(t, i)}`}>
              {t.price.toLocaleString()}
            </span>
            <span className="text-right font-mono">
              {t.qty.toLocaleString()}
            </span>
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
          <div className="flex items-center justify-center py-8">
            <p className="text-xs text-[var(--text-muted)]">No trades yet</p>
          </div>
        )}
      </div>
    </div>
  );
};

export default TradeFeed;
