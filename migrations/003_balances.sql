DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'asset_type') THEN
        CREATE TYPE asset_type AS ENUM ('USDC', 'ETH', 'BTC', 'SOL');
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS balances (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id),
    asset asset_type NOT NULL,
    available_balance BIGINT NOT NULL DEFAULT 0,
    locked_balance BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (account_id, asset)
);

CREATE INDEX IF NOT EXISTS idx_balances_account_id ON balances (account_id);
CREATE INDEX IF NOT EXISTS idx_balances_account_asset ON balances (account_id, asset);
