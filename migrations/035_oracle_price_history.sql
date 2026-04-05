-- Price history for TWAP calculations and anomaly detection.
CREATE TABLE IF NOT EXISTS oracle_price_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    market_id UUID NOT NULL REFERENCES markets(id),
    price BIGINT NOT NULL,
    source TEXT NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_oracle_history_market_time
    ON oracle_price_history (market_id, fetched_at DESC);

-- Add source column to market_prices if missing (idempotent)
-- Already exists from 017 migration, but ensure index:
CREATE INDEX IF NOT EXISTS idx_market_prices_updated
    ON market_prices (updated_at DESC);
