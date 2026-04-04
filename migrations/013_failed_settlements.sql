CREATE TABLE IF NOT EXISTS failed_settlements (
    id UUID PRIMARY KEY,
    trade_id UUID NOT NULL REFERENCES trades(id),
    retry_count INT NOT NULL DEFAULT 0,
    last_error TEXT,
    next_retry_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    permanently_failed BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (trade_id)
);

CREATE INDEX IF NOT EXISTS idx_failed_settlements_next_retry
    ON failed_settlements (next_retry_at)
    WHERE permanently_failed = FALSE;
