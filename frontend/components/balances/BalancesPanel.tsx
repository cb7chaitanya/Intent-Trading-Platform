import React, { useEffect, useState } from "react";
import { Lock, RefreshCw } from "lucide-react";
import { getAccounts, getBalances } from "@/lib/api";
import { useAuth } from "@/contexts/AuthProvider";

interface Balance {
  id: string;
  account_id: string;
  asset: string;
  available_balance: number;
  locked_balance: number;
}

const ASSET_COLORS: Record<string, string> = {
  ETH: "text-blue-400",
  BTC: "text-orange-400",
  SOL: "text-purple-400",
  USDC: "text-green-400",
};

const BalancesPanel: React.FC = () => {
  const { user } = useAuth();
  const [balances, setBalances] = useState<Balance[]>([]);
  const [refreshing, setRefreshing] = useState(false);

  const load = async () => {
    if (!user?.user_id) return;
    setRefreshing(true);
    try {
      const accounts = await getAccounts(user.user_id);
      if (accounts?.length > 0) {
        const data = await getBalances(accounts[0].id);
        setBalances(data || []);
      }
    } catch {
      setBalances([]);
    }
    setRefreshing(false);
  };

  useEffect(() => {
    load();
  }, [user?.user_id]);

  // Auto-refresh every 15s
  useEffect(() => {
    const interval = setInterval(load, 15000);
    return () => clearInterval(interval);
  }, [user?.user_id]);

  const totalValue = balances.reduce(
    (sum, b) => sum + b.available_balance + b.locked_balance,
    0
  );

  return (
    <div className="rounded-xl border bg-surface-1 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--text-muted)]">
          Balances
        </h3>
        <button
          onClick={load}
          disabled={refreshing}
          className="btn-ghost !p-1 rounded"
          aria-label="Refresh balances"
        >
          <RefreshCw
            size={12}
            className={refreshing ? "animate-spin" : ""}
          />
        </button>
      </div>

      <div className="divide-y">
        {balances.map((b) => (
          <div
            key={b.id}
            className="flex items-center justify-between px-3 py-2 hover:bg-surface-2 transition-colors"
          >
            <div className="flex items-center gap-2">
              <span
                className={`text-xs font-bold ${ASSET_COLORS[b.asset] || "text-[var(--text-primary)]"}`}
              >
                {b.asset}
              </span>
            </div>
            <div className="text-right">
              <p className="text-xs font-mono font-medium">
                {b.available_balance.toLocaleString()}
              </p>
              {b.locked_balance > 0 && (
                <div className="flex items-center justify-end gap-0.5 text-[10px] text-yellow-500">
                  <Lock size={8} />
                  <span>{b.locked_balance.toLocaleString()}</span>
                </div>
              )}
            </div>
          </div>
        ))}
        {balances.length === 0 && (
          <p className="text-center text-[11px] text-[var(--text-muted)] py-4">
            {user ? "No balances" : "Sign in to view"}
          </p>
        )}
      </div>
    </div>
  );
};

export default BalancesPanel;
