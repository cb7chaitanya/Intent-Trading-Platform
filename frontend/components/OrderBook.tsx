import React, { useEffect, useState, useRef } from "react";
import { getOrderbook } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";

interface PriceLevel {
  price: number;
  qty: number;
}

interface OrderBookProps {
  marketId: string;
}

const OrderBook: React.FC<OrderBookProps> = ({ marketId }) => {
  const [bids, setBids] = useState<PriceLevel[]>([]);
  const [asks, setAsks] = useState<PriceLevel[]>([]);
  const { addListener } = useWebSocket();
  const prevSpreadRef = useRef<number | null>(null);

  useEffect(() => {
    getOrderbook(marketId)
      .then((data) => {
        setBids(data.bids || []);
        setAsks(data.asks || []);
      })
      .catch(() => {});
  }, [marketId]);

  useEffect(() => {
    return addListener("OrderBook", (data) => {
      if (data.market_id === marketId) {
        setBids(data.bids || []);
        setAsks(data.asks || []);
      }
    });
  }, [marketId, addListener]);

  const maxQty = Math.max(
    ...bids.map((b) => b.qty),
    ...asks.map((a) => a.qty),
    1
  );

  const spread =
    asks.length > 0 && bids.length > 0
      ? asks[asks.length - 1]?.price - bids[0]?.price
      : null;

  return (
    <div className="card space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">Order Book</h3>
        {spread !== null && (
          <span className="text-xs text-[var(--text-muted)]">
            Spread: {spread}
          </span>
        )}
      </div>

      {/* Header */}
      <div className="grid grid-cols-3 text-xs text-[var(--text-muted)] px-2">
        <span>Price</span>
        <span className="text-right">Size</span>
        <span className="text-right">Total</span>
      </div>

      {/* Asks (reversed so lowest ask is at bottom) */}
      <div className="space-y-px max-h-48 overflow-y-auto flex flex-col-reverse">
        {asks.slice(0, 12).map((a, i) => {
          const cumulative = asks
            .slice(0, i + 1)
            .reduce((s, x) => s + x.qty, 0);
          return (
            <div
              key={`a-${i}`}
              className="relative grid grid-cols-3 px-2 py-1 text-xs rounded-sm group hover:bg-surface-2 transition-colors"
            >
              <div
                className="absolute inset-0 rounded-sm bg-down/5"
                style={{ width: `${(a.qty / maxQty) * 100}%`, right: 0, left: "auto" }}
              />
              <span className="relative text-down font-mono">{a.price}</span>
              <span className="relative text-right font-mono">{a.qty}</span>
              <span className="relative text-right font-mono text-[var(--text-muted)]">
                {cumulative}
              </span>
            </div>
          );
        })}
      </div>

      {/* Spread divider */}
      {spread !== null && (
        <div className="flex items-center gap-2 px-2">
          <div className="flex-1 border-t border-dashed" />
          <span className="text-xs font-mono font-medium text-brand-400">
            {bids[0]?.price || "—"}
          </span>
          <div className="flex-1 border-t border-dashed" />
        </div>
      )}

      {/* Bids */}
      <div className="space-y-px max-h-48 overflow-y-auto">
        {bids.slice(0, 12).map((b, i) => {
          const cumulative = bids
            .slice(0, i + 1)
            .reduce((s, x) => s + x.qty, 0);
          return (
            <div
              key={`b-${i}`}
              className="relative grid grid-cols-3 px-2 py-1 text-xs rounded-sm group hover:bg-surface-2 transition-colors"
            >
              <div
                className="absolute inset-0 rounded-sm bg-up/5"
                style={{ width: `${(b.qty / maxQty) * 100}%`, right: 0, left: "auto" }}
              />
              <span className="relative text-up font-mono">{b.price}</span>
              <span className="relative text-right font-mono">{b.qty}</span>
              <span className="relative text-right font-mono text-[var(--text-muted)]">
                {cumulative}
              </span>
            </div>
          );
        })}
      </div>

      {bids.length === 0 && asks.length === 0 && (
        <p className="text-center text-sm text-[var(--text-muted)] py-6">
          No orders yet
        </p>
      )}
    </div>
  );
};

export default OrderBook;
