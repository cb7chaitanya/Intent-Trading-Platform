import React, { useEffect, useState, useMemo } from "react";
import { getOrderbook } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";
import type { PriceLevel } from "@/lib/ws";

interface OrderBookProps {
  marketId: string;
  onPriceClick?: (price: number) => void;
  rows?: number;
}

const OrderBook: React.FC<OrderBookProps> = ({
  marketId,
  onPriceClick,
  rows = 14,
}) => {
  const [bids, setBids] = useState<PriceLevel[]>([]);
  const [asks, setAsks] = useState<PriceLevel[]>([]);
  const { addListener } = useWebSocket();

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

  const slicedAsks = useMemo(() => asks.slice(0, rows).reverse(), [asks, rows]);
  const slicedBids = useMemo(() => bids.slice(0, rows), [bids, rows]);

  const maxQty = useMemo(
    () =>
      Math.max(
        ...slicedBids.map((b) => b.qty),
        ...slicedAsks.map((a) => a.qty),
        1
      ),
    [slicedBids, slicedAsks]
  );

  const bidCumulative = useMemo(() => {
    let sum = 0;
    return slicedBids.map((b) => {
      sum += b.qty;
      return sum;
    });
  }, [slicedBids]);

  const askCumulative = useMemo(() => {
    const sums: number[] = [];
    let sum = 0;
    // reversed asks: accumulate from bottom (lowest ask) up
    for (let i = slicedAsks.length - 1; i >= 0; i--) {
      sum += slicedAsks[i].qty;
      sums[i] = sum;
    }
    return sums;
  }, [slicedAsks]);

  const spread =
    asks.length > 0 && bids.length > 0 ? asks[0].price - bids[0].price : null;
  const spreadPct =
    spread != null && bids[0]?.price > 0
      ? ((spread / bids[0].price) * 100).toFixed(2)
      : null;
  const midPrice =
    asks.length > 0 && bids.length > 0
      ? (asks[0].price + bids[0].price) / 2
      : null;

  const Row: React.FC<{
    price: number;
    qty: number;
    cumulative: number;
    side: "bid" | "ask";
  }> = ({ price, qty, cumulative, side }) => (
    <div
      onClick={() => onPriceClick?.(price)}
      className="relative grid grid-cols-3 px-2 py-[3px] text-[11px] cursor-pointer hover:bg-surface-2 transition-colors"
    >
      <div
        className={`absolute inset-y-0 right-0 ${
          side === "bid" ? "bg-up/8" : "bg-down/8"
        }`}
        style={{ width: `${(qty / maxQty) * 100}%` }}
      />
      <span
        className={`relative font-mono ${
          side === "bid" ? "text-up" : "text-down"
        }`}
      >
        {price.toLocaleString()}
      </span>
      <span className="relative text-right font-mono">
        {qty.toLocaleString()}
      </span>
      <span className="relative text-right font-mono text-[var(--text-muted)]">
        {cumulative.toLocaleString()}
      </span>
    </div>
  );

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-3 py-2 border-b">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--text-muted)]">
          Order Book
        </h3>
      </div>

      {/* Column headers */}
      <div className="grid grid-cols-3 px-2 py-1.5 text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
        <span>Price</span>
        <span className="text-right">Size</span>
        <span className="text-right">Total</span>
      </div>

      {/* Asks */}
      <div className="flex-1 overflow-hidden flex flex-col justify-end">
        {slicedAsks.map((a, i) => (
          <Row
            key={`a-${i}`}
            price={a.price}
            qty={a.qty}
            cumulative={askCumulative[i]}
            side="ask"
          />
        ))}
      </div>

      {/* Spread / mid price */}
      <div className="flex items-center justify-between px-2 py-1.5 border-y bg-surface-2/50">
        {midPrice != null ? (
          <>
            <span className="text-sm font-mono font-bold text-brand-400">
              {midPrice.toLocaleString()}
            </span>
            <span className="text-[10px] text-[var(--text-muted)]">
              Spread: {spread?.toLocaleString()} ({spreadPct}%)
            </span>
          </>
        ) : (
          <span className="text-xs text-[var(--text-muted)]">--</span>
        )}
      </div>

      {/* Bids */}
      <div className="flex-1 overflow-hidden">
        {slicedBids.map((b, i) => (
          <Row
            key={`b-${i}`}
            price={b.price}
            qty={b.qty}
            cumulative={bidCumulative[i]}
            side="bid"
          />
        ))}
      </div>

      {bids.length === 0 && asks.length === 0 && (
        <div className="flex-1 flex items-center justify-center">
          <p className="text-xs text-[var(--text-muted)]">No orders yet</p>
        </div>
      )}
    </div>
  );
};

export default OrderBook;
