CREATE TABLE IF NOT EXISTS solver_positions (
    solver_id TEXT NOT NULL,
    asset TEXT NOT NULL,
    position BIGINT NOT NULL DEFAULT 0,
    avg_entry_price BIGINT NOT NULL DEFAULT 0,
    realized_pnl BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (solver_id, asset)
);
