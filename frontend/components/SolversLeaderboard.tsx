import React, { useEffect, useState, useRef } from "react";
import { Trophy, RefreshCw } from "lucide-react";
import { getTopSolvers } from "@/lib/api";

interface Solver {
  id: string;
  name: string;
  successful_trades: number;
  failed_trades: number;
  total_volume: number;
  reputation_score: number;
}

interface SolversLeaderboardProps {
  limit?: number;
  compact?: boolean;
  currentUserId?: string;
  refreshInterval?: number;
}

const REFRESH_MS = 10_000;

const SolversLeaderboard: React.FC<SolversLeaderboardProps> = ({
  limit = 10,
  compact = false,
  currentUserId,
  refreshInterval = REFRESH_MS,
}) => {
  const [solvers, setSolvers] = useState<Solver[]>([]);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval>>();

  const fetch = async () => {
    setRefreshing(true);
    try {
      const data = await getTopSolvers(limit);
      setSolvers(data || []);
      setLastUpdated(new Date());
    } catch {}
    setRefreshing(false);
  };

  useEffect(() => {
    fetch();
    timerRef.current = setInterval(fetch, refreshInterval);
    return () => clearInterval(timerRef.current);
  }, [limit, refreshInterval]);

  const medalColor = (i: number) => {
    if (i === 0) return "text-yellow-400";
    if (i === 1) return "text-gray-300";
    if (i === 2) return "text-amber-600";
    return "text-[var(--text-muted)]";
  };

  return (
    <div className="card space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Trophy size={16} className="text-yellow-400" />
          <h3 className="text-sm font-semibold">Top Solvers</h3>
        </div>
        <div className="flex items-center gap-2">
          {lastUpdated && (
            <span className="text-[10px] text-[var(--text-muted)]">
              {lastUpdated.toLocaleTimeString()}
            </span>
          )}
          <button
            onClick={fetch}
            disabled={refreshing}
            className="btn-ghost !p-1 rounded"
            aria-label="Refresh"
          >
            <RefreshCw
              size={12}
              className={refreshing ? "animate-spin" : ""}
            />
          </button>
        </div>
      </div>

      {/* Header */}
      {!compact && solvers.length > 0 && (
        <div className="grid grid-cols-12 gap-2 px-3 text-[10px] font-medium uppercase tracking-wider text-[var(--text-muted)]">
          <span className="col-span-1">#</span>
          <span className="col-span-4">Solver</span>
          <span className="col-span-3 text-center">Record</span>
          <span className="col-span-2 text-right">Volume</span>
          <span className="col-span-2 text-right">Score</span>
        </div>
      )}

      <div className="space-y-0.5">
        {solvers.map((s, i) => {
          const total = s.successful_trades + s.failed_trades;
          const winRate = total > 0 ? (s.successful_trades / total) * 100 : 0;
          const isCurrentUser = currentUserId === s.id;

          return (
            <div
              key={s.id}
              className={`rounded-lg px-3 py-2 transition-colors ${
                isCurrentUser
                  ? "bg-brand-600/10 ring-1 ring-brand-500/30"
                  : "hover:bg-surface-2"
              }`}
            >
              {compact ? (
                <div className="flex items-center gap-3">
                  <span
                    className={`w-5 text-center text-xs font-bold ${medalColor(i)}`}
                  >
                    {i + 1}
                  </span>
                  <span className="flex-1 text-sm font-medium truncate">
                    {s.name}
                    {isCurrentUser && (
                      <span className="ml-1.5 badge badge-info text-[10px]">
                        You
                      </span>
                    )}
                  </span>
                  <span className="text-sm font-mono font-semibold text-brand-400">
                    {s.reputation_score.toFixed(2)}
                  </span>
                </div>
              ) : (
                <div className="grid grid-cols-12 gap-2 items-center">
                  <span
                    className={`col-span-1 text-center text-xs font-bold ${medalColor(i)}`}
                  >
                    {i + 1}
                  </span>
                  <div className="col-span-4 min-w-0">
                    <p className="text-sm font-medium truncate">
                      {s.name}
                      {isCurrentUser && (
                        <span className="ml-1.5 badge badge-info text-[10px]">
                          You
                        </span>
                      )}
                    </p>
                  </div>
                  <div className="col-span-3 text-center text-xs">
                    <span className="text-up">{s.successful_trades}W</span>
                    <span className="text-[var(--text-muted)]"> / </span>
                    <span className="text-down">{s.failed_trades}L</span>
                    <span className="text-[var(--text-muted)] ml-1">
                      ({winRate.toFixed(0)}%)
                    </span>
                  </div>
                  <span className="col-span-2 text-right text-xs text-[var(--text-muted)] font-mono">
                    {s.total_volume.toLocaleString()}
                  </span>
                  <span className="col-span-2 text-right text-sm font-mono font-semibold text-brand-400">
                    {s.reputation_score.toFixed(2)}
                  </span>
                </div>
              )}
            </div>
          );
        })}

        {solvers.length === 0 && (
          <p className="text-center text-sm text-[var(--text-muted)] py-6">
            No solvers yet
          </p>
        )}
      </div>
    </div>
  );
};

export default SolversLeaderboard;
