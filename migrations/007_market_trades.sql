CREATE TABLE IF NOT EXISTS market_trades (
    id UUID PRIMARY KEY,
    market_id UUID NOT NULL REFERENCES markets(id),
    buyer_account_id UUID NOT NULL REFERENCES accounts(id),
    seller_account_id UUID NOT NULL REFERENCES accounts(id),
    price BIGINT NOT NULL,
    qty BIGINT NOT NULL,
    fee BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_market_trades_market_id ON market_trades (market_id);
CREATE INDEX IF NOT EXISTS idx_market_trades_created_at ON market_trades (market_id, created_at);
