DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'trade_status') THEN
        CREATE TYPE trade_status AS ENUM ('pending', 'settled', 'failed');
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS trades (
    id UUID PRIMARY KEY,
    buyer_account_id UUID NOT NULL REFERENCES accounts(id),
    seller_account_id UUID NOT NULL REFERENCES accounts(id),
    solver_account_id UUID NOT NULL REFERENCES accounts(id),
    asset_in asset_type NOT NULL,
    asset_out asset_type NOT NULL,
    amount_in BIGINT NOT NULL,
    amount_out BIGINT NOT NULL,
    platform_fee BIGINT NOT NULL DEFAULT 0,
    solver_fee BIGINT NOT NULL DEFAULT 0,
    status trade_status NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_trades_buyer ON trades (buyer_account_id);
CREATE INDEX IF NOT EXISTS idx_trades_seller ON trades (seller_account_id);
CREATE INDEX IF NOT EXISTS idx_trades_status ON trades (status);
CREATE INDEX IF NOT EXISTS idx_trades_created_at ON trades (created_at);
