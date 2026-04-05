-- Per-solver performance statistics, updated after each fill settlement.
CREATE TABLE IF NOT EXISTS solver_stats (
    solver_id UUID PRIMARY KEY REFERENCES solvers(id),
    total_auctions_entered BIGINT NOT NULL DEFAULT 0,
    total_auctions_won     BIGINT NOT NULL DEFAULT 0,
    total_fills            BIGINT NOT NULL DEFAULT 0,
    total_settled          BIGINT NOT NULL DEFAULT 0,
    total_failed           BIGINT NOT NULL DEFAULT 0,
    total_volume           BIGINT NOT NULL DEFAULT 0,
    total_profit           BIGINT NOT NULL DEFAULT 0,
    sum_latency_ms         BIGINT NOT NULL DEFAULT 0,
    sum_slippage_bps       BIGINT NOT NULL DEFAULT 0,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Materialised leaderboard index — fast ORDER BY for the leaderboard endpoint.
CREATE INDEX IF NOT EXISTS idx_solver_stats_volume
    ON solver_stats (total_volume DESC);
CREATE INDEX IF NOT EXISTS idx_solver_stats_profit
    ON solver_stats (total_profit DESC);
