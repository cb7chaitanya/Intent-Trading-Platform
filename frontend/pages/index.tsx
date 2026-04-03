import React, { useEffect, useState } from "react";
import { Activity, TrendingUp, Users } from "lucide-react";
import { getMarkets, getBalances } from "@/lib/api";
import MarketCard from "@/components/MarketCard";
import SolversLeaderboard from "@/components/SolversLeaderboard";
import BalanceCards from "@/components/BalanceCards";

interface Market {
  id: string;
  base_asset: string;
  quote_asset: string;
  tick_size: number;
  min_order_size: number;
  fee_rate: number;
}

export default function Dashboard() {
  const [markets, setMarkets] = useState<Market[]>([]);
  const [balances, setBalances] = useState<any[]>([]);
  const [accountId, setAccountId] = useState("");

  useEffect(() => {
    getMarkets()
      .then((data) => setMarkets(data || []))
      .catch(() => {});
  }, []);

  const loadBalances = () => {
    if (!accountId.trim()) return;
    getBalances(accountId)
      .then((data) => setBalances(data || []))
      .catch(() => setBalances([]));
  };

  return (
    <div className="space-y-6 max-w-7xl mx-auto animate-fade-in">
      {/* Stats row */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="card flex items-center gap-4">
          <div className="h-10 w-10 rounded-xl bg-brand-600/10 flex items-center justify-center">
            <TrendingUp size={20} className="text-brand-400" />
          </div>
          <div>
            <p className="text-xs text-[var(--text-muted)]">Active Markets</p>
            <p className="text-xl font-bold">{markets.length}</p>
          </div>
        </div>
        <div className="card flex items-center gap-4">
          <div className="h-10 w-10 rounded-xl bg-up/10 flex items-center justify-center">
            <Activity size={20} className="text-up" />
          </div>
          <div>
            <p className="text-xs text-[var(--text-muted)]">Platform Status</p>
            <p className="text-xl font-bold text-up">Online</p>
          </div>
        </div>
        <div className="card flex items-center gap-4">
          <div className="h-10 w-10 rounded-xl bg-purple-500/10 flex items-center justify-center">
            <Users size={20} className="text-purple-400" />
          </div>
          <div>
            <p className="text-xs text-[var(--text-muted)]">Solvers Active</p>
            <p className="text-xl font-bold">—</p>
          </div>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Markets */}
        <div className="lg:col-span-2 space-y-4">
          <h2 className="text-lg font-semibold">Markets</h2>
          {markets.length === 0 ? (
            <div className="card text-center py-12">
              <TrendingUp
                size={32}
                className="mx-auto text-[var(--text-muted)] mb-2"
              />
              <p className="text-sm text-[var(--text-muted)]">
                No markets available yet
              </p>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {markets.map((m) => (
                <MarketCard key={m.id} market={m} />
              ))}
            </div>
          )}
        </div>

        {/* Solvers */}
        <div>
          <h2 className="text-lg font-semibold mb-4">Leaderboard</h2>
          <SolversLeaderboard limit={10} />
        </div>
      </div>

      {/* Balances quick-view */}
      <div className="space-y-3">
        <h2 className="text-lg font-semibold">Quick Balance Check</h2>
        <div className="flex gap-2">
          <input
            className="input max-w-sm"
            placeholder="Enter Account ID..."
            value={accountId}
            onChange={(e) => setAccountId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && loadBalances()}
          />
          <button onClick={loadBalances} className="btn-primary">
            Load
          </button>
        </div>
        {balances.length > 0 && <BalanceCards balances={balances} />}
      </div>
    </div>
  );
}
