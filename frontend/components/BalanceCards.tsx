import React from "react";
import { Wallet, Lock } from "lucide-react";

interface Balance {
  id: string;
  account_id: string;
  asset: string;
  available_balance: number;
  locked_balance: number;
}

interface BalanceCardsProps {
  balances: Balance[];
}

const ASSET_COLORS: Record<string, string> = {
  ETH: "from-blue-500/20 to-blue-600/5",
  BTC: "from-orange-500/20 to-orange-600/5",
  SOL: "from-purple-500/20 to-purple-600/5",
  USDC: "from-green-500/20 to-green-600/5",
};

const BalanceCards: React.FC<BalanceCardsProps> = ({ balances }) => {
  if (balances.length === 0) {
    return (
      <p className="text-sm text-[var(--text-muted)] text-center py-6">
        No balances found
      </p>
    );
  }

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {balances.map((b) => (
        <div
          key={b.id}
          className={`card bg-gradient-to-br ${
            ASSET_COLORS[b.asset] || "from-surface-2 to-surface-1"
          }`}
        >
          <div className="flex items-center justify-between mb-2">
            <span className="text-sm font-semibold">{b.asset}</span>
            <Wallet size={14} className="text-[var(--text-muted)]" />
          </div>
          <p className="text-xl font-mono font-bold">
            {b.available_balance.toLocaleString()}
          </p>
          {b.locked_balance > 0 && (
            <div className="flex items-center gap-1 mt-1 text-xs text-yellow-500">
              <Lock size={10} />
              <span>{b.locked_balance.toLocaleString()} locked</span>
            </div>
          )}
        </div>
      ))}
    </div>
  );
};

export default BalanceCards;
