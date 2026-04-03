import React, { useEffect, useState } from "react";
import { getMarkets } from "@/lib/api";
import MarketCard from "@/components/MarketCard";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

export default function MarketsPage() {
  const [markets, setMarkets] = useState<Market[]>([]);

  useEffect(() => {
    getMarkets()
      .then((data) => setMarkets(data || []))
      .catch(() => {});
  }, []);

  return (
    <div className="space-y-6 max-w-5xl mx-auto animate-fade-in">
      <h1 className="text-2xl font-bold">Markets</h1>
      {markets.length === 0 ? (
        <div className="card text-center py-12 text-[var(--text-muted)]">
          No markets available
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
          {markets.map((m) => (
            <MarketCard key={m.id} market={m} />
          ))}
        </div>
      )}
    </div>
  );
}
