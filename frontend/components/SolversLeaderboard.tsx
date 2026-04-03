import React, { useEffect, useState } from "react";
import { Trophy } from "lucide-react";
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
}

const SolversLeaderboard: React.FC<SolversLeaderboardProps> = ({
  limit = 10,
  compact = false,
}) => {
  const [solvers, setSolvers] = useState<Solver[]>([]);

  useEffect(() => {
    getTopSolvers(limit)
      .then((data) => setSolvers(data || []))
      .catch(() => {});
  }, [limit]);

  const medalColor = (i: number) => {
    if (i === 0) return "text-yellow-400";
    if (i === 1) return "text-gray-300";
    if (i === 2) return "text-amber-600";
    return "text-[var(--text-muted)]";
  };

  return (
    <div className="card space-y-3">
      <div className="flex items-center gap-2">
        <Trophy size={16} className="text-yellow-400" />
        <h3 className="text-sm font-semibold">Top Solvers</h3>
      </div>

      <div className="space-y-1">
        {solvers.map((s, i) => {
          const total = s.successful_trades + s.failed_trades;
          const winRate = total > 0 ? (s.successful_trades / total) * 100 : 0;

          return (
            <div
              key={s.id}
              className="flex items-center gap-3 rounded-lg px-3 py-2 hover:bg-surface-2 transition-colors"
            >
              <span
                className={`w-5 text-center text-xs font-bold ${medalColor(i)}`}
              >
                {i + 1}
              </span>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium truncate">{s.name}</p>
                {!compact && (
                  <p className="text-xs text-[var(--text-muted)]">
                    {s.successful_trades}W / {s.failed_trades}L (
                    {winRate.toFixed(0)}%)
                  </p>
                )}
              </div>
              {!compact && (
                <span className="text-xs text-[var(--text-muted)] font-mono">
                  {s.total_volume.toLocaleString()}
                </span>
              )}
              <span className="text-sm font-mono font-semibold text-brand-400">
                {s.reputation_score.toFixed(2)}
              </span>
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
