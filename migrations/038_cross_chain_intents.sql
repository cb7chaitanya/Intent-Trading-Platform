-- Cross-chain intent support.

-- Add chain fields to intents.
ALTER TABLE intents ADD COLUMN IF NOT EXISTS source_chain TEXT NOT NULL DEFAULT 'ethereum';
ALTER TABLE intents ADD COLUMN IF NOT EXISTS destination_chain TEXT NOT NULL DEFAULT 'ethereum';
ALTER TABLE intents ADD COLUMN IF NOT EXISTS cross_chain BOOLEAN NOT NULL DEFAULT FALSE;

-- Add execution strategy to bids.
ALTER TABLE bids ADD COLUMN IF NOT EXISTS execution_strategy TEXT;

-- Escrow status for cross-chain settlements.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'leg_status') THEN
        CREATE TYPE leg_status AS ENUM (
            'pending', 'escrowed', 'executing', 'confirmed', 'failed', 'refunded'
        );
    END IF;
END $$;

-- Each cross-chain settlement has two legs (source + destination).
CREATE TABLE IF NOT EXISTS cross_chain_legs (
    id UUID PRIMARY KEY,
    intent_id UUID NOT NULL REFERENCES intents(id),
    fill_id UUID NOT NULL,
    leg_index SMALLINT NOT NULL,           -- 0 = source, 1 = destination
    chain TEXT NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT NOT NULL,
    token_mint TEXT,
    amount BIGINT NOT NULL,
    tx_hash TEXT,
    status leg_status NOT NULL DEFAULT 'pending',
    error TEXT,
    timeout_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ,
    UNIQUE (fill_id, leg_index)
);

CREATE INDEX IF NOT EXISTS idx_cross_chain_legs_intent ON cross_chain_legs (intent_id);
CREATE INDEX IF NOT EXISTS idx_cross_chain_legs_status ON cross_chain_legs (status)
    WHERE status IN ('pending', 'escrowed', 'executing');
CREATE INDEX IF NOT EXISTS idx_cross_chain_legs_timeout ON cross_chain_legs (timeout_at)
    WHERE status NOT IN ('confirmed', 'refunded');
