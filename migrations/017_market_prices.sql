CREATE TABLE IF NOT EXISTS market_prices (
    market_id UUID NOT NULL REFERENCES markets(id),
    price BIGINT NOT NULL,
    source TEXT NOT NULL DEFAULT 'mock',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (market_id)
);
