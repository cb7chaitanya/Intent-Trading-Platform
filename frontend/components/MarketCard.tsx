import React from "react";
import Link from "next/link";
import { TrendingUp } from "lucide-react";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

interface MarketCardProps {
  market: Market;
}

const MarketCard: React.FC<MarketCardProps> = ({ market }) => {
  return (
    <Link href={`/market/${market.id}`}>
      <div className="card-hover group">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <div className="h-8 w-8 rounded-full bg-brand-600/10 flex items-center justify-center">
              <TrendingUp size={14} className="text-brand-400" />
            </div>
            <span className="font-semibold">
              {market.base_asset}/{market.quote_asset}
            </span>
          </div>
          <span className="badge badge-info">
            {(market.fee_rate * 100).toFixed(2)}%
          </span>
        </div>
        <div className="grid grid-cols-2 gap-2 text-xs text-[var(--text-muted)]">
          <div>
            <span className="block text-[var(--text-secondary)]">Tick</span>
            <span className="font-mono">{market.tick_size}</span>
          </div>
          <div>
            <span className="block text-[var(--text-secondary)]">Min Size</span>
            <span className="font-mono">{market.min_order_size}</span>
          </div>
        </div>
      </div>
    </Link>
  );
};

export default MarketCard;
