CREATE TABLE IF NOT EXISTS solvers (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    successful_trades BIGINT NOT NULL DEFAULT 0,
    failed_trades BIGINT NOT NULL DEFAULT 0,
    total_volume BIGINT NOT NULL DEFAULT 0,
    reputation_score DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_solvers_reputation ON solvers (reputation_score DESC);
