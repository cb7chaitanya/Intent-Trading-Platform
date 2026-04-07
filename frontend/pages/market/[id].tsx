import React, { useEffect, useState, useCallback } from "react";
import { useRouter } from "next/router";
import { Wifi, WifiOff } from "lucide-react";
import { getMarket, getOraclePrice } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";
import MarketSelector from "@/components/market-selector/MarketSelector";
import CandlestickChart from "@/components/chart/CandlestickChart";
import OrderBook from "@/components/orderbook/OrderBook";
import TradeFeed from "@/components/trade-feed/TradeFeed";
import IntentForm from "@/components/intent-form/IntentForm";
import BalancesPanel from "@/components/balances/BalancesPanel";
import OpenOrders from "@/components/open-orders/OpenOrders";
import { CrossChainForm } from "@/components/cross-chain";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

export default function TradingPage() {
  const router = useRouter();
  const { id } = router.query;
  const marketId = id as string;

  const [market, setMarket] = useState<Market | null>(null);
  const [oraclePrice, setOraclePrice] = useState<number | null>(null);
  const [selectedPrice, setSelectedPrice] = useState<number | undefined>();
  const [tradeMode, setTradeMode] = useState<"single" | "cross">("single");
  const { subscribe, unsubscribe, connected } = useWebSocket();

  useEffect(() => {
    if (!marketId) return;
    getMarket(marketId)
      .then(setMarket)
      .catch(() => {});
    getOraclePrice(marketId)
      .then((data) => setOraclePrice(data?.price ?? null))
      .catch(() => {});
  }, [marketId]);

  useEffect(() => {
    if (!marketId || !connected) return;
    subscribe(marketId);
    return () => unsubscribe(marketId);
  }, [marketId, connected, subscribe, unsubscribe]);

  // Refresh oracle price periodically
  useEffect(() => {
    if (!marketId) return;
    const interval = setInterval(() => {
      getOraclePrice(marketId)
        .then((data) => setOraclePrice(data?.price ?? null))
        .catch(() => {});
    }, 5000);
    return () => clearInterval(interval);
  }, [marketId]);

  const handlePriceClick = useCallback((price: number) => {
    setSelectedPrice(price);
  }, []);

  const handleOrderPlaced = useCallback(() => {
    // OpenOrders component will refresh via its own listener
  }, []);

  if (!marketId) return null;

  return (
    <div className="h-[calc(100vh-3.5rem)] flex flex-col overflow-hidden animate-fade-in -m-6">
      {/* Market header bar */}
      <div className="flex items-center justify-between px-4 py-2 border-b bg-surface-1 shrink-0">
        <div className="flex items-center gap-4">
          <MarketSelector currentMarketId={marketId} />

          {/* Price stats */}
          <div className="hidden md:flex items-center gap-6 text-xs">
            {oraclePrice != null && (
              <div>
                <span className="text-[var(--text-muted)] mr-1">Oracle</span>
                <span className="font-mono font-medium text-brand-400">
                  {oraclePrice.toLocaleString()}
                </span>
              </div>
            )}
            {market && (
              <>
                <div>
                  <span className="text-[var(--text-muted)] mr-1">
                    Tick Size
                  </span>
                  <span className="font-mono">{market.tick_size}</span>
                </div>
                <div>
                  <span className="text-[var(--text-muted)] mr-1">
                    Min Size
                  </span>
                  <span className="font-mono">{market.min_order_size}</span>
                </div>
                <div>
                  <span className="text-[var(--text-muted)] mr-1">Fee</span>
                  <span className="font-mono">
                    {(market.fee_rate * 100).toFixed(2)}%
                  </span>
                </div>
              </>
            )}
          </div>
        </div>

        {/* Connection status */}
        <div className="flex items-center gap-1.5">
          {connected ? (
            <Wifi size={14} className="text-up" />
          ) : (
            <WifiOff size={14} className="text-down" />
          )}
          <span
            className={`text-[10px] font-medium ${
              connected ? "text-up" : "text-down"
            }`}
          >
            {connected ? "LIVE" : "OFFLINE"}
          </span>
        </div>
      </div>

      {/* Main grid */}
      <div className="flex-1 min-h-0 grid grid-cols-12 grid-rows-[1fr_auto]">
        {/* Left: Order Book */}
        <div className="col-span-3 xl:col-span-2 border-r overflow-hidden">
          <OrderBook
            marketId={marketId}
            onPriceClick={handlePriceClick}
          />
        </div>

        {/* Center: Chart + Open Orders */}
        <div className="col-span-6 xl:col-span-7 flex flex-col overflow-hidden">
          {/* Chart */}
          <div className="flex-1 min-h-0">
            <CandlestickChart marketId={marketId} />
          </div>

          {/* Trade feed (horizontal below chart) */}
          <div className="h-48 border-t overflow-hidden">
            <TradeFeed
              marketId={marketId}
              onPriceClick={handlePriceClick}
            />
          </div>
        </div>

        {/* Right: Intent Form + Balances */}
        <div className="col-span-3 border-l flex flex-col overflow-hidden">
          {/* Trade mode tabs */}
          <div className="flex border-b shrink-0">
            <button
              onClick={() => setTradeMode("single")}
              className={`flex-1 py-2 text-[11px] font-medium transition-colors ${
                tradeMode === "single"
                  ? "text-[var(--text-primary)] border-b-2 border-brand-500"
                  : "text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
              }`}
            >
              Trade
            </button>
            <button
              onClick={() => setTradeMode("cross")}
              className={`flex-1 py-2 text-[11px] font-medium transition-colors ${
                tradeMode === "cross"
                  ? "text-[var(--text-primary)] border-b-2 border-brand-500"
                  : "text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
              }`}
            >
              Cross-Chain
            </button>
          </div>

          {/* Form (switches based on mode) */}
          <div className="flex-1 min-h-0 overflow-hidden">
            {tradeMode === "single" ? (
              <IntentForm
                marketId={marketId}
                baseAsset={market?.base_asset}
                quoteAsset={market?.quote_asset}
                tickSize={market?.tick_size}
                minOrderSize={market?.min_order_size}
                onOrderPlaced={handleOrderPlaced}
                initialPrice={selectedPrice}
              />
            ) : (
              <CrossChainForm
                baseAsset={market?.base_asset}
                quoteAsset={market?.quote_asset}
                onOrderPlaced={handleOrderPlaced}
              />
            )}
          </div>

          {/* Balances */}
          <div className="shrink-0 border-t">
            <BalancesPanel />
          </div>
        </div>

        {/* Bottom: Open Orders / History (full width) */}
        <div className="col-span-12 border-t max-h-56 overflow-hidden">
          <OpenOrders marketId={marketId} />
        </div>
      </div>
    </div>
  );
}
