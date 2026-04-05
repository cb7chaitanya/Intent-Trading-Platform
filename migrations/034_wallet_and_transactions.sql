-- Platform-managed wallets for on-chain settlement.
CREATE TABLE IF NOT EXISTS wallets (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL,
    address TEXT NOT NULL,
    chain TEXT NOT NULL DEFAULT 'ethereum',
    encrypted_key BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wallets_address ON wallets (address);
CREATE INDEX IF NOT EXISTS idx_wallets_account ON wallets (account_id);

-- On-chain transaction tracking.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'tx_status') THEN
        CREATE TYPE tx_status AS ENUM (
            'pending', 'submitted', 'confirmed', 'failed', 'dropped'
        );
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS transactions (
    id UUID PRIMARY KEY,
    fill_id UUID REFERENCES fills(id),
    from_address TEXT NOT NULL,
    to_address TEXT NOT NULL,
    chain TEXT NOT NULL DEFAULT 'ethereum',
    tx_hash TEXT,
    amount BIGINT NOT NULL,
    asset TEXT NOT NULL,
    status tx_status NOT NULL DEFAULT 'pending',
    gas_price BIGINT,
    gas_used BIGINT,
    block_number BIGINT,
    confirmations INT NOT NULL DEFAULT 0,
    error TEXT,
    submitted_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_transactions_fill ON transactions (fill_id);
CREATE INDEX IF NOT EXISTS idx_transactions_status ON transactions (status) WHERE status IN ('pending', 'submitted');
CREATE INDEX IF NOT EXISTS idx_transactions_hash ON transactions (tx_hash) WHERE tx_hash IS NOT NULL;
