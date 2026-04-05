import React, { useState, useEffect, useRef } from "react";
import { useRouter } from "next/router";
import { ChevronDown, Search } from "lucide-react";
import { getMarkets, getOraclePrices } from "@/lib/api";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

interface OraclePrice {
  market_id: string;
  price: number;
  updated_at: string;
}

interface MarketSelectorProps {
  currentMarketId: string;
  onMarketChange?: (market: Market) => void;
}

const MarketSelector: React.FC<MarketSelectorProps> = ({
  currentMarketId,
  onMarketChange,
}) => {
  const router = useRouter();
  const [markets, setMarkets] = useState<Market[]>([]);
  const [prices, setPrices] = useState<Record<string, number>>({});
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    getMarkets()
      .then((data) => setMarkets(data || []))
      .catch(() => {});
    getOraclePrices()
      .then((data: OraclePrice[]) => {
        const map: Record<string, number> = {};
        (data || []).forEach((p) => {
          map[p.market_id] = p.price;
        });
        setPrices(map);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, []);

  const current = markets.find((m) => m.id === currentMarketId);
  const pair = current
    ? `${current.base_asset}/${current.quote_asset}`
    : "Select Market";
  const currentPrice = prices[currentMarketId];

  const filtered = markets.filter((m) => {
    const label = `${m.base_asset}/${m.quote_asset}`.toLowerCase();
    return label.includes(search.toLowerCase());
  });

  const handleSelect = (m: Market) => {
    setOpen(false);
    setSearch("");
    onMarketChange?.(m);
    router.push(`/market/${m.id}`);
  };

  return (
    <div ref={dropdownRef} className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 rounded-lg px-3 py-2 hover:bg-surface-2 transition-colors"
      >
        <span className="text-lg font-bold">{pair}</span>
        {currentPrice != null && (
          <span className="text-sm font-mono text-brand-400 ml-1">
            {currentPrice.toLocaleString()}
          </span>
        )}
        <ChevronDown
          size={16}
          className={`text-[var(--text-muted)] transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="absolute top-full left-0 mt-1 w-72 rounded-xl border bg-surface-1 shadow-xl z-50 animate-fade-in overflow-hidden">
          <div className="p-2 border-b">
            <div className="relative">
              <Search
                size={14}
                className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-muted)]"
              />
              <input
                className="input !pl-8 !py-1.5 !text-xs"
                placeholder="Search markets..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                autoFocus
              />
            </div>
          </div>
          <div className="max-h-64 overflow-y-auto py-1">
            {filtered.map((m) => {
              const label = `${m.base_asset}/${m.quote_asset}`;
              const isActive = m.id === currentMarketId;
              const price = prices[m.id];
              return (
                <button
                  key={m.id}
                  onClick={() => handleSelect(m)}
                  className={`w-full flex items-center justify-between px-3 py-2 text-sm transition-colors ${
                    isActive
                      ? "bg-brand-600/10 text-brand-400"
                      : "hover:bg-surface-2"
                  }`}
                >
                  <span className="font-medium">{label}</span>
                  {price != null && (
                    <span className="font-mono text-xs text-[var(--text-muted)]">
                      {price.toLocaleString()}
                    </span>
                  )}
                </button>
              );
            })}
            {filtered.length === 0 && (
              <p className="text-center text-xs text-[var(--text-muted)] py-4">
                No markets found
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default MarketSelector;
